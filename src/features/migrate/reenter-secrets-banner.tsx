import type { LegacyMigrationStatus } from "../../generated/bindings";

interface ReenterSecretsBannerProps {
  status: LegacyMigrationStatus | null;
}

export function ReenterSecretsBanner({ status }: ReenterSecretsBannerProps) {
  if (!status?.reenterSecrets) {
    return null;
  }
  return (
    <p className="reenter-secrets-banner" role="status">
      需要重新填写密钥
      <a href="#/services">去服务页填写</a>
    </p>
  );
}
