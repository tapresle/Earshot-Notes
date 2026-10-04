import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import App from "./App";

describe("App shell", () => {
  it("renders the meetings sidebar, workspace, and transcript panel", () => {
    render(<App />);

    expect(screen.getByRole("navigation", { name: "Meetings" })).toBeInTheDocument();
    expect(screen.getByRole("main")).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Transcript" })).toBeInTheDocument();
  });

  it("shows the first-meeting empty state", () => {
    render(<App />);

    expect(screen.getByRole("heading", { level: 1, name: "Start your first meeting" })).toBeInTheDocument();
  });
});
