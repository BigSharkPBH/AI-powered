import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppearanceSettings } from "./appearance-settings";
import { initializeTheme, THEME_STORAGE_KEY } from "./theme";

describe("appearance preferences", () => {
  let media: { matches: boolean; addEventListener: ReturnType<typeof vi.fn>; removeEventListener: ReturnType<typeof vi.fn> };
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    window.localStorage.clear();
    media = { matches: false, addEventListener: vi.fn(), removeEventListener: vi.fn() };
    vi.stubGlobal("matchMedia", vi.fn(() => media));
  });
  afterEach(() => {
    cleanup();
    dispose?.();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    document.documentElement.removeAttribute("data-theme");
    document.documentElement.style.colorScheme = "";
  });

  it("uses the system preference before the first page renders", () => {
    media.matches = true;
    dispose = initializeTheme();
    render(<AppearanceSettings />);
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect((screen.getByRole("radio", { name: "跟随系统" }) as HTMLInputElement).checked).toBe(true);
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBeNull();
  });

  it("applies and saves a selection, then restores it on startup", () => {
    dispose = initializeTheme();
    render(<AppearanceSettings />);
    fireEvent.click(screen.getByRole("radio", { name: "深色" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
    dispose();
    act(() => { dispose = initializeTheme(); });
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect((screen.getByRole("radio", { name: "深色" }) as HTMLInputElement).checked).toBe(true);
  });

  it("responds to system changes only when following the system", () => {
    dispose = initializeTheme();
    render(<AppearanceSettings />);
    const notifySystemChange = () => media.addEventListener.mock.calls[0][1]();
    media.matches = true;
    act(notifySystemChange);
    expect(document.documentElement.dataset.theme).toBe("dark");
    fireEvent.click(screen.getByRole("radio", { name: "浅色" }));
    act(notifySystemChange);
    expect(document.documentElement.dataset.theme).toBe("light");
    fireEvent.click(screen.getByRole("radio", { name: "跟随系统" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("falls back safely for an invalid saved preference", () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, "unexpected");
    dispose = initializeTheme();
    render(<AppearanceSettings />);
    expect((screen.getByRole("radio", { name: "跟随系统" }) as HTMLInputElement).checked).toBe(true);
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("keeps the theme usable when persistence is unavailable", () => {
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => { throw new Error("unavailable"); });
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("unavailable"); });
    dispose = initializeTheme();
    render(<AppearanceSettings />);
    fireEvent.click(screen.getByRole("radio", { name: "深色" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect((screen.getByRole("radio", { name: "深色" }) as HTMLInputElement).checked).toBe(true);
  });

  it("synchronizes preference changes from another window", () => {
    dispose = initializeTheme();
    render(<AppearanceSettings />);
    act(() => window.dispatchEvent(new StorageEvent("storage", { key: THEME_STORAGE_KEY, newValue: "dark" })));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect((screen.getByRole("radio", { name: "深色" }) as HTMLInputElement).checked).toBe(true);
  });

  it("removes system listeners during cleanup", () => {
    dispose = initializeTheme();
    dispose();
    expect(media.removeEventListener).toHaveBeenCalledWith("change", media.addEventListener.mock.calls[0][1]);
  });
});
