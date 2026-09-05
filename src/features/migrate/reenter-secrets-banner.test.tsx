import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ReenterSecretsBanner } from "./reenter-secrets-banner";

describe("ReenterSecretsBanner", () => {
  afterEach(cleanup);

  it("shows 需要重新填写密钥 when reenterSecrets is true", () => {
    render(
      <ReenterSecretsBanner
        status={{ applied: true, reenterSecrets: true, omitted: [".env"] }}
      />,
    );
    const banner = screen.getByRole("status");
    expect(banner.textContent).toContain("需要重新填写密钥");
    expect(banner.textContent).toContain("服务");
  });

  it("is hidden when reenterSecrets is false", () => {
    const { container } = render(
      <ReenterSecretsBanner
        status={{ applied: true, reenterSecrets: false, omitted: [] }}
      />,
    );
    expect(screen.queryByText("需要重新填写密钥")).toBeNull();
    expect(container.textContent).not.toContain("需要重新填写密钥");
  });

  it("is hidden when status is missing", () => {
    const { container } = render(<ReenterSecretsBanner status={null} />);
    expect(container.textContent).not.toContain("需要重新填写密钥");
  });

  it("does not use window.prompt", () => {
    const promptSpy = vi.spyOn(window, "prompt");
    render(
      <ReenterSecretsBanner
        status={{ applied: true, reenterSecrets: true, omitted: [] }}
      />,
    );
    expect(promptSpy).not.toHaveBeenCalled();
    promptSpy.mockRestore();
  });
});
