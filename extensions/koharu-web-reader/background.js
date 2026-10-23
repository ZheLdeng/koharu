const KOHARU_ENDPOINT = "http://127.0.0.1:31785/v1/translate";

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
	if (message?.type !== "koharu-translate-image") return undefined;

	translateImage(message)
		.then((dataUrl) => sendResponse({ ok: true, dataUrl }))
		.catch((error) => sendResponse({ ok: false, error: error.message }));
	return true;
});

async function translateImage({ imageUrl, pageUrl }) {
	const url = new URL(imageUrl);
	if (url.protocol !== "https:" && url.protocol !== "http:") {
		throw new Error("This reader uses a temporary image URL; this page needs a site-specific adapter.");
	}

	const original = await fetch(url.href, { credentials: "include" });
	if (!original.ok) throw new Error(`Could not read the comic image (${original.status}).`);
	const image = await original.blob();
	if (!image.type.startsWith("image/")) throw new Error("The reader did not return an image.");
	if (image.size > 64 * 1024 * 1024) throw new Error("The image exceeds Koharu's 64 MB browser limit.");

	const controller = new AbortController();
	const timeout = setTimeout(() => controller.abort(), 180_000);
	try {
		const translated = await fetch(KOHARU_ENDPOINT, {
			method: "POST",
			headers: { "Content-Type": image.type },
			body: image,
			signal: controller.signal,
		});
		if (!translated.ok) {
			const detail = (await translated.text()).slice(0, 240);
			throw new Error(detail || `Koharu returned ${translated.status}.`);
		}
		const bytes = new Uint8Array(await translated.arrayBuffer());
		return `data:image/png;base64,${toBase64(bytes)}`;
	} catch (error) {
		if (error.name === "AbortError") throw new Error("Koharu took longer than three minutes; try this image again.");
		if (error instanceof TypeError) {
			throw new Error("Cannot reach Koharu. Start the modified Koharu 0.83.5 build first.");
		}
		throw error;
	} finally {
		clearTimeout(timeout);
	}
}

function toBase64(bytes) {
	let binary = "";
	const chunkSize = 0x8000;
	for (let offset = 0; offset < bytes.length; offset += chunkSize) {
		binary += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
	}
	return btoa(binary);
}
