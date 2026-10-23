const button = document.querySelector("#translate");
const status = document.querySelector("#status");

button.addEventListener("click", async () => {
	button.disabled = true;
	status.textContent = "Sending the visible image to Koharu…";
	try {
		const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
		if (!tab?.id || !/^https?:/.test(tab.url || "")) {
			throw new Error("Open a manga reader page in this tab first.");
		}
		const result = await chrome.tabs.sendMessage(tab.id, { type: "koharu-translate-visible" });
		if (!result?.ok) throw new Error(result?.error || "The page could not be processed.");
		status.textContent = result.message;
	} catch (error) {
		status.textContent = error.message.includes("Receiving end")
			? "Refresh this manga tab, then try again."
			: error.message;
	} finally {
		button.disabled = false;
	}
});
