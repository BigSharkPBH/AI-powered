import { WorkspaceSession } from "../../features/session/workspace-session";
import { PageShell } from "../page-shell";

export function WorkspacePage() {
  return (
    <div className="workspace-page">
      <PageShell id="workspace" />
      <WorkspaceSession />
    </div>
  );
}
