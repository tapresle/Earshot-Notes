import { describe, expect, it, vi } from "vitest";
import { errorMessage } from "./api";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(), save: vi.fn() }));

describe("errorMessage", () => {
  it("names a missing meeting", () => {
    expect(errorMessage({ kind: "notFound", missing: "meeting", message: "meeting 7 doesn't exist" })).toBe(
      "That meeting no longer exists.",
    );
  });

  it("names a missing local AI model", () => {
    expect(errorMessage({ kind: "notFound", missing: "model", message: "model local-abc doesn't exist" })).toBe(
      "That local AI model is no longer installed.",
    );
  });

  it("uses the backend's message when it doesn't say what is missing", () => {
    expect(errorMessage({ kind: "notFound", message: "export folder doesn't exist" })).toBe("Export folder doesn't exist.");
  });
});
