import { FormEvent, useState } from "react";

import { importLegacySource } from "../../api/commands";

export function LegacyImportPanel() {
  const [path, setPath] = useState("");
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    setBusy(true);
    try {
      const result = await importLegacySource(path.trim());
      if (result.ok) {
        setMessage(`已导入 ${result.data.sessions} 个会话，${result.data.turns} 轮`);
      } else {
        setMessage(`${result.error.code}：${result.error.message}`);
      }
    } catch {
      setMessage("IPC_UNAVAILABLE：无法导入旧会话");
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="service-panel settings-section" aria-labelledby="legacy-import-heading">
      <h2 className="section-heading" id="legacy-import-heading">导入旧会话</h2>
      <p className="configuration-description">从旧版本的数据目录导入会话记录。</p>
      <form className="service-form configuration-migration-form" onSubmit={(event) => void submit(event)}>
        <label htmlFor="legacy-source-path">旧数据目录</label>
        <input
          id="legacy-source-path"
          name="legacySourcePath"
          value={path}
          onChange={(event) => setPath(event.target.value)}
          placeholder="例如 E:\\old-install 或 .desktop-runtime"
        />
        <button className="button-primary" type="submit" disabled={busy || path.trim().length === 0}>
          导入旧会话
        </button>
      </form>
      {message ? <p className="services-message" role="status">{message}</p> : null}
    </section>
  );
}
