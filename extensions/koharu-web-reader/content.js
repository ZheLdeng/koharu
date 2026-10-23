const translations = new WeakMap();

chrome.runtime.onMessage.addListener((message, _sender, sendResponse) => {
	if (message?.type !== "koharu-translate-visible") return undefined;
	translateVisibleImage()
		.then((result) => sendResponse({ ok: true, ...result }))
		.catch((error) => sendResponse({ ok: false, error: error.message }));
	return true;
});

async function translateVisibleImage() {
	const image = findVisibleImage();
	if (!image) throw new Error("No large comic image is visible. Scroll to a page and try again.");

	const existing = translations.get(image);
	if (existing?.translated) {
		restoreOriginal(image, existing);
		return { message: "Original image restored." };
	}

	const original = existing ?? {
		src: image.getAttribute("src"),
		srcset: image.getAttribute("srcset"),
		sizes: image.getAttribute("sizes"),
		currentSrc: image.currentSrc,
		translated: false,
	};
	if (!existing) translations.set(image, original);
	const imageUrl = original.currentSrc || original.src;
	if (!imageUrl) throw new Error("The visible comic image has no readable source URL.");

	const result = await chrome.runtime.sendMessage({
		type: "koharu-translate-image",
		imageUrl,
		pageUrl: location.href,
	});
	if (!result?.ok) throw new Error(result?.error || "Koharu translation failed.");

	original.translatedUrl = result.dataUrl;
	original.translated = true;
	image.removeAttribute("srcset");
	image.setAttribute("src", result.dataUrl);
	return { message: "Translated image replaced the page image. Click again to restore the original." };
}

function restoreOriginal(image, state) {
	state.translated = false;
	if (state.src === null) image.removeAttribute("src");
	else image.setAttribute("src", state.src);
	if (state.srcset === null) image.removeAttribute("srcset");
	else image.setAttribute("srcset", state.srcset);
	if (state.sizes === null) image.removeAttribute("sizes");
	else image.setAttribute("sizes", state.sizes);
}

function findVisibleImage() {
	const viewportWidth = document.documentElement.clientWidth;
	const viewportHeight = document.documentElement.clientHeight;
	let best = null;
	let bestArea = 0;

	for (const image of document.images) {
		if (!image.complete || image.naturalWidth < 300 || image.naturalHeight < 250) continue;
		const ratio = image.naturalWidth / image.naturalHeight;
		if (ratio > 5 || ratio < 0.18) continue;
		const rect = image.getBoundingClientRect();
		const width = Math.max(0, Math.min(rect.right, viewportWidth) - Math.max(rect.left, 0));
		const height = Math.max(0, Math.min(rect.bottom, viewportHeight) - Math.max(rect.top, 0));
		const area = width * height;
		if (area > bestArea) {
			best = image;
			bestArea = area;
		}
	}

	return bestArea >= 20_000 ? best : null;
}
