// Renders the real app with a fake backend, for the committed screenshots.
// Open /preview.html?scene=<name> while `vite` runs. Never part of the app build.
import React from "react";
import ReactDOM from "react-dom/client";
import { mockIPC } from "@tauri-apps/api/mocks";
import App from "../App";
import { scenes } from "./scenes";

const name = new URLSearchParams(window.location.search).get("scene") ?? "";
const scene = scenes[name];
const root = document.getElementById("root") as HTMLElement;

if (scene) {
  const backend = scene();
  mockIPC((command, args) => backend.handle(command, args), { shouldMockEvents: true });
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
} else {
  root.textContent = `Unknown scene "${name}". Scenes: ${Object.keys(scenes).join(", ")}.`;
}
