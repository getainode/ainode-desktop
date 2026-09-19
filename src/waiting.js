// Waiting page: shown while no master address answers. The Rust side does
// the probing and navigates this window to the AINode UI when one answers;
// this script only keeps the text current.
(function () {
  const { invoke } = window.__TAURI__.core;

  const primary = document.getElementById("primary");
  const also = document.getElementById("also");
  const reason = document.getElementById("reason");

  async function refresh() {
    try {
      const s = await invoke("get_status_view");
      if (!s.configured) {
        primary.textContent = "your master AINode";
        also.textContent = "Nothing is set up yet. Open Settings and enter its address.";
        reason.textContent = "";
        return;
      }
      primary.textContent = s.primary;
      also.textContent = s.alternate ? "also trying " + s.alternate : "";
      reason.textContent = s.last_error || "";
    } catch (e) {
      reason.textContent = String(e);
    }
  }

  document.getElementById("retry").addEventListener("click", async () => {
    reason.textContent = "Retrying...";
    await invoke("retry_now");
    setTimeout(refresh, 2500);
  });
  document.getElementById("settings").addEventListener("click", () => invoke("open_settings"));

  refresh();
  setInterval(refresh, 1000);
})();
