# Koharu Web Reader (experimental)

This unpacked Chrome extension translates the largest comic image intersecting the active viewport. It sends the source image to the Koharu 0.83.5 desktop process at `127.0.0.1:31785`; Koharu runs its existing configured pipeline in an isolated in-memory page and returns a PNG. The extension replaces the selected `<img>` in place and can restore the original image by clicking the button again.

## Install for development

1. Build and run the modified Koharu 0.83.5 application. The local endpoint starts after Koharu initializes its pipeline.
2. Open `chrome://extensions`, turn on Developer mode, and choose **Load unpacked**.
3. Select this directory.
4. Open or refresh an HTTP(S) manga reader page, scroll the desired image into view, then click the extension button.

The reader must expose the comic page as a network-backed `<img>`. Canvas-only viewers, `blob:` image URLs, cross-origin CDNs that reject extension fetches, and sites that replace image elements after translation need a site-specific adapter. This first version translates one visible image per click; it does not auto-translate a whole chapter.

The endpoint binds only to loopback, accepts requests only from this extension's fixed Chrome origin, checks the Host header, accepts image content types only, and limits uploads to 64 MB and 40 million pixels. The extension sends images only to this local Koharu endpoint. Translation providers behave according to Koharu's saved configuration.
