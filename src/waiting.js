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
      // Say that the fleet is being tried too. A cluster's other nodes serve
      // the same models, so an outage on the configured address is not an
      // outage, and the page should not imply that it is.
      const parts = [];
      if (s.alternate) parts.push("also trying " + s.alternate);
      if (s.fleet_count) {
        parts.push(
          s.fleet_count === 1
            ? "and 1 other node of the fleet"
            : "and " + s.fleet_count + " other nodes of the fleet"
        );
      }
      also.textContent = parts.join(" ");
      // A node that answered "give me a key" is up, and a certificate this
      // machine will not verify has a fix; either one belongs on screen ahead of
      // "nothing answered", which is what last_error says.
      reason.textContent = s.needs_attention || s.last_error || "";
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
