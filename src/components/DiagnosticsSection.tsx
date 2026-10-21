import { save } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { LifeBuoy } from "lucide-react";
import { useState } from "react";

import { ipc } from "../lib/ipc";
import { useApp } from "../store/useApp";
import { Button, Section } from "./ui";

/**
 * One zip for a bug report: a summary of the setup, the virtual file
 * system's logs and mapping, and the frame generation package's recent logs.
 */
export function DiagnosticsSection() {
  const toast = useApp((s) => s.toast);
  const [working, setWorking] = useState(false);

  async function collect() {
    const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
    const path = await save({
      title: "Save diagnostics",
      defaultPath: `sbmm-diagnostics-${stamp}.zip`,
      filters: [{ name: "Zip archive", extensions: ["zip"] }],
    });
    if (!path) return;

    setWorking(true);
    try {
      const written = await ipc.collectDiagnostics(path);
      toast("success", "Diagnostics saved. Attach the zip to your report.");
      void revealItemInDir(written);
    } catch (error) {
      toast("error", String(error));
    } finally {
      setWorking(false);
    }
  }

  return (
    <Section
      title="Diagnostics"
      hint="Collects everything needed to look into a problem into one zip: the setup, your mod list, and the launch, virtual file system and frame generation logs. Your Nexus key is never included; folder paths are, and can contain your Windows user name."
    >
      <Button variant="outline" disabled={working} onClick={() => void collect()}>
        <LifeBuoy size={13} />
        {working ? "Collecting…" : "Collect diagnostics"}
      </Button>
    </Section>
  );
}
