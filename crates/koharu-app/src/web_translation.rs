use std::{collections::BTreeMap, convert::Infallible, io::Cursor, sync::Arc};

use anyhow::{Context as _, Result, bail};
use bytes::Bytes;
use http_body_util::{BodyExt as _, Full, Limited};
use hyper::{
    Request, Response, StatusCode,
    body::Incoming,
    server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::TokioIo;
use image::{ExtendedColorType, ImageEncoder as _, codecs::png::{CompressionType, FilterType, PngEncoder}};
use koharu_pipeline::{Committer, Operation, RunStatus, StageOutput};
use koharu_rasterizer::RasterOptions;
use koharu_scene::{AssetInput, AssetMetadata, AssetRole, At, PageDraft, Session, Snapshot};
use tauri::{AppHandle, Manager as _};
use tauri_runtime_cef::CefRuntime;
use tokio::net::TcpListener;

use koharu_desktop::Desktop;
use koharu_renderer::Renderer;

const LISTEN_ADDRESS: &str = "127.0.0.1:31785";
const EXPECTED_HOST: &str = "127.0.0.1:31785";
const EXTENSION_ORIGIN: &str = "chrome-extension://hflfcachpljkeaphdlibamecnmcmeoko";
const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;
const MAX_IMAGE_EDGE: u32 = 32_768;
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;

type ApiResponse = Response<Full<Bytes>>;

pub(crate) async fn start(app: AppHandle<CefRuntime>) -> Result<()> {
    let listener = TcpListener::bind(LISTEN_ADDRESS)
        .await
        .with_context(|| format!("failed to start the local manga translation endpoint at {LISTEN_ADDRESS}"))?;
    tracing::info!(address = LISTEN_ADDRESS, "local manga translation endpoint ready");

    drop(tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::error!(%error, "local manga translation endpoint accept failed");
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    continue;
                }
            };
            let app = app.clone();
            drop(tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let app = app.clone();
                    async move { Ok::<_, Infallible>(handle_request(request, app).await) }
                });
                if let Err(error) = http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await
                {
                    tracing::debug!(%error, "local manga translation connection ended");
                }
            }));
        }
    }));
    Ok(())
}

async fn handle_request(request: Request<Incoming>, app: AppHandle<CefRuntime>) -> ApiResponse {
    let origin = request
        .headers()
        .get("origin")
        .and_then(|value| value.to_str().ok());
    if origin != Some(EXTENSION_ORIGIN)
        || request
            .headers()
            .get("host")
            .and_then(|value| value.to_str().ok())
            != Some(EXPECTED_HOST)
    {
        return response(StatusCode::FORBIDDEN, "text/plain", Bytes::from_static(b"forbidden"), false);
    }

    if request.method() == hyper::Method::OPTIONS && request.uri().path() == "/v1/translate" {
        let mut response = response(StatusCode::NO_CONTENT, "text/plain", Bytes::new(), true);
        response.headers_mut().insert(
            "access-control-allow-methods",
            hyper::header::HeaderValue::from_static("POST, OPTIONS"),
        );
        response.headers_mut().insert(
            "access-control-allow-headers",
            hyper::header::HeaderValue::from_static("content-type"),
        );
        response.headers_mut().insert(
            "access-control-max-age",
            hyper::header::HeaderValue::from_static("600"),
        );
        return response;
    }

    if request.method() != hyper::Method::POST || request.uri().path() != "/v1/translate" {
        return response(StatusCode::NOT_FOUND, "text/plain", Bytes::from_static(b"not found"), true);
    }
    if !request
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("image/"))
    {
        return response(StatusCode::UNSUPPORTED_MEDIA_TYPE, "text/plain", Bytes::from_static(b"send an image"), true);
    }
    if request
        .headers()
        .get(hyper::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .is_some_and(|length| length > MAX_UPLOAD_BYTES)
    {
        return response(StatusCode::PAYLOAD_TOO_LARGE, "text/plain", Bytes::from_static(b"image is too large"), true);
    }

    let body = match Limited::new(request.into_body(), MAX_UPLOAD_BYTES).collect().await {
        Ok(body) => body.to_bytes(),
        Err(error) => {
            tracing::warn!(%error, "browser image upload was rejected");
            return response(StatusCode::PAYLOAD_TOO_LARGE, "text/plain", Bytes::from_static(b"image is too large"), true);
        }
    };

    match translate_image(body, app).await {
        Ok(image) => response(StatusCode::OK, "image/png", image, true),
        Err(error) => {
            tracing::error!(%error, "browser image translation failed");
            response(StatusCode::INTERNAL_SERVER_ERROR, "text/plain", Bytes::from_static(b"Koharu could not translate this image"), true)
        }
    }
}

async fn translate_image(source: Bytes, app: AppHandle<CefRuntime>) -> Result<Bytes> {
    if source.is_empty() {
        bail!("image upload is empty");
    }
    let format = image::guess_format(&source).context("unsupported image format")?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP) {
        bail!("only PNG, JPEG, and WebP images are supported");
    }
    let (width, height) = image::ImageReader::with_format(Cursor::new(source.as_ref()), format)
        .into_dimensions()
        .context("could not read image dimensions")?;
    if width == 0
        || height == 0
        || width > MAX_IMAGE_EDGE
        || height > MAX_IMAGE_EDGE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        bail!("image dimensions exceed the local processing limit");
    }

    let mut session = Session::memory().await?;
    let mut page = None;
    let patch = session.snapshot().patch(|edit| {
        let page_id = edit.add_page(
            PageDraft::new("browser-image", f64::from(width), f64::from(height)),
            At::End,
        )?;
        edit.set_asset(
            page_id,
            &AssetRole::new("source")?,
            AssetInput::new(
                Arc::<[u8]>::from(source.as_ref()),
                image_media_type(format),
                AssetMetadata {
                    width: Some(width),
                    height: Some(height),
                    attributes: BTreeMap::new(),
                },
            ),
        )?;
        page = Some(page_id);
        Ok(())
    })?;
    session.commit(patch).await?;
    let page = page.context("temporary page was not created")?;

    let pipeline = app.state::<koharu_pipeline::Pipeline>().inner().clone();
    let snapshot = session.snapshot();
    let mut committer = MemoryCommitter(&mut session);
    let report = pipeline
        .execute(
            snapshot,
            koharu_pipeline::Request {
                operation: Operation::Full,
                scope: koharu_pipeline::Scope::Pages(vec![page]),
                ..Default::default()
            },
            &mut committer,
        )
        .await?;
    if report.status != RunStatus::Completed {
        bail!("Koharu stopped before finishing the page");
    }

    let snapshot = session.snapshot();
    let desktop = app.state::<Desktop>();
    let renderer = Renderer::new()?;
    let rasterizer = desktop.rasterizer().await?;
    let frame = renderer.render(&snapshot, page).await?;
    let raster = tokio_rayon::spawn(move || {
        rasterizer.rasterize(&frame.raster_frame()?, RasterOptions::default())
    })
    .await??;

    let encoded = tokio_rayon::spawn(move || -> Result<Vec<u8>> {
        let mut encoded = Vec::new();
        PngEncoder::new_with_quality(&mut encoded, CompressionType::Fast, FilterType::Adaptive)
            .write_image(
                raster.image.as_raw(),
                raster.image.width(),
                raster.image.height(),
                ExtendedColorType::Rgba8,
            )?;
        Ok(encoded)
    })
    .await??;
    if encoded.len() > MAX_OUTPUT_BYTES {
        bail!("translated image exceeds the 32 MB browser response limit");
    }
    Ok(Bytes::from(encoded))
}

struct MemoryCommitter<'a>(&'a mut Session);

#[async_trait::async_trait]
impl Committer for MemoryCommitter<'_> {
    async fn commit(&mut self, output: StageOutput) -> Result<Snapshot> {
        Ok(self.0.commit(output.patch).await?.snapshot)
    }
}

fn image_media_type(format: image::ImageFormat) -> &'static str {
    match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::WebP => "image/webp",
        _ => "application/octet-stream",
    }
}

fn response(status: StatusCode, content_type: &'static str, body: Bytes, cors: bool) -> ApiResponse {
    let mut response = Response::new(Full::new(body));
    *response.status_mut() = status;
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static(content_type),
    );
    response.headers_mut().insert(
        hyper::header::CACHE_CONTROL,
        hyper::header::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        "x-content-type-options",
        hyper::header::HeaderValue::from_static("nosniff"),
    );
    if cors {
        response.headers_mut().insert(
            "access-control-allow-origin",
            hyper::header::HeaderValue::from_static(EXTENSION_ORIGIN),
        );
        response.headers_mut().insert(
            hyper::header::VARY,
            hyper::header::HeaderValue::from_static("Origin"),
        );
    }
    response
}

