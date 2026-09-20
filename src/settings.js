// Settings screen: two addresses, a Test button for each, Save.
(function () {
  const { invoke } = window.__TAURI__.core;

  const primary = document.getElementById("primary");
  const alternate = document.getElementById("alternate");
  const error = document.getElementById("error");
  const save = document.getElementById("save");
  const cancel = document.getElementById("cancel");

  function setResult(id, cls, text) {
    const el = document.getElementById("result-" + id);
    el.className = "result " + cls;
    el.textContent = text;
  }

  async function test(id) {
    const input = document.getElementById(id);
    const button = document.getElementById("test-" + id);
    const value = input.value.trim();
    if (!value) {
      setResult(id, "bad", id === "alternate" ? "Nothing to test." : "Enter an address first.");
      return;
    }
    button.disabled = true;
    setResult(id, "busy", "Testing " + value + "...");
    try {
      const r = await invoke("test_address", { address: value });
      const model = r.model ? " · " + r.model.split("/").pop() : "";
      setResult(id, "ok", r.node_name + " · AINode " + r.version + model);
      input.value = r.address;
    } catch (e) {
      setResult(id, "bad", "No answer from " + value + ": " + String(e));
    } finally {
      button.disabled = false;
    }
  }

  async function doSave() {
    error.textContent = "";
    save.disabled = true;
    try {
      await invoke("save_settings", {
        primary: primary.value,
        alternate: alternate.value,
      });
      await invoke("close_self");
    } catch (e) {
      error.textContent = String(e);
      save.disabled = false;
    }
  }

  document.getElementById("test-primary").addEventListener("click", () => test("primary"));
  document.getElementById("test-alternate").addEventListener("click", () => test("alternate"));
  save.addEventListener("click", doSave);
  cancel.addEventListener("click", () => invoke("close_self"));

  for (const input of [primary, alternate]) {
    input.addEventListener("keydown", (ev) => {
      if (ev.key === "Enter") {
        ev.preventDefault();
        doSave();
      } else if (ev.key === "Escape") {
        invoke("close_self");
      }
    });
    input.addEventListener("input", () => {
      error.textContent = "";
      setResult(input.id, "", "");
    });
  }

  // Which node is answering, and how many fallbacks are saved. Read-only: the
  // fleet list comes from the nodes themselves, so there is nothing to edit.
  const serving = document.getElementById("serving");

  function fleetLine(count) {
    if (!count) return "";
    return count === 1
      ? " · 1 other node of the fleet saved as a fallback"
      : " · " + count + " other nodes of the fleet saved as fallbacks";
  }

  async function refreshServing() {
    try {
      const s = await invoke("get_status_view");
      if (!s.configured) {
        serving.className = "result";
        serving.textContent = "";
        return;
      }
      if (!s.master) {
        serving.className = "result bad";
        serving.textContent = (s.last_error || "Not connected") + fleetLine(s.fleet_count);
        return;
      }
      const who = s.serving_via || "Connected to " + (s.serving_name || s.master);
      serving.className = "result ok";
      serving.textContent = who + " (" + s.master + ")" + fleetLine(s.fleet_count);
    } catch (e) {
      serving.className = "result";
      serving.textContent = "";
    }
  }

  invoke("get_settings").then((s) => {
    primary.value = s.primary || "";
    alternate.value = s.alternate || "";
    primary.focus();
  });
  refreshServing();
  setInterval(refreshServing, 2000);
})();
