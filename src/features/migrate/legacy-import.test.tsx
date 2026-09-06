import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import * as commands from "../../api/commands";
import { LegacyImportPanel } from "./legacy-import";

vi.mock("../../api/commands", () => ({
  importLegacySource: vi.fn(),
}));

describe("LegacyImportPanel", () => {
  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("imports the typed absolute path and shows session counts", async () => {
    vi.mocked(commands.importLegacySource).mockResolvedValue({
      ok: true,
      data: { sessions: 1, turns: 2 },
    });
    render(<LegacyImportPanel />);
    fireEvent.change(screen.getByLabelText("旧数据目录"), {
      target: { value: "E:\\\\old-install" },
    });
    fireEvent.click(screen.getByRole("button", { name: "导入旧会话" }));
    expect((await screen.findByRole("status")).textContent).toContain("已导入 1 个会话，2 轮");
    expect(commands.importLegacySource).toHaveBeenCalledWith("E:\\\\old-install");
  });

  it("surfaces a failed import without calling fetch", async () => {
    const fetchSpy = vi.spyOn(globalThis, "fetch");
    vi.mocked(commands.importLegacySource).mockResolvedValue({
      ok: false,
      error: {
        code: "MIGRATE_PAYLOAD_INVALID",
        message: "旧会话数据无法解析",
        requestId: "req",
        retryable: false,
      },
    });
    render(<LegacyImportPanel />);
    fireEvent.change(screen.getByLabelText("旧数据目录"), {
      target: { value: "E:\\\\broken" },
    });
    fireEvent.click(screen.getByRole("button", { name: "导入旧会话" }));
    expect((await screen.findByRole("status")).textContent).toContain("MIGRATE_PAYLOAD_INVALID");
    expect(fetchSpy).not.toHaveBeenCalled();
    fetchSpy.mockRestore();
  });
});
