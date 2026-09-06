import { useEffect, useState } from "react";

import { getLegacyMigrationStatus } from "../../api/commands";
import { LegacyImportPanel } from "../../features/migrate/legacy-import";
import { ReenterSecretsBanner } from "../../features/migrate/reenter-secrets-banner";
import { RoleEditor } from "../../features/roles/role-editor";
import type { LegacyMigrationStatus } from "../../generated/bindings";
import { PageShell } from "../page-shell";

export function SettingsPage() {
  const [migration, setMigration] = useState<LegacyMigrationStatus | null>(null);

  useEffect(() => {
    void getLegacyMigrationStatus()
      .then((result) => {
        if (result.ok) setMigration(result.data);
      })
      .catch(() => {
        setMigration(null);
      });
  }, []);

  return (
    <>
      <PageShell id="settings" />
      <ReenterSecretsBanner status={migration} />
      <LegacyImportPanel />
      <RoleEditor />
    </>
  );
}
