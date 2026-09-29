// The one script on mesimon.dev: a Copy button beside each command. Without
// it the buttons stay hidden and the commands are ordinary selectable text.
for (const button of document.querySelectorAll(".cmd .copy")) {
  const text = button.previousElementSibling.textContent;
  if (!navigator.clipboard) continue;
  button.hidden = false;
  button.addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText(text);
      button.textContent = "Copied";
    } catch {
      button.textContent = "Select it";
    }
    setTimeout(() => (button.textContent = "Copy"), 1500);
  });
}
