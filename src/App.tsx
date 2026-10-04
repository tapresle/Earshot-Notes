import "./styles.css";

export default function App() {
  return (
    <div className="app-shell">
      <nav className="sidebar" aria-label="Meetings">
        <h2 className="pane-heading">Meetings</h2>
        <p className="empty-hint">Your meetings appear here.</p>
      </nav>

      <main className="workspace">
        <div className="empty-state">
          <h1>Start your first meeting</h1>
          <p>Take notes while the app transcribes on this device. Nothing leaves your computer.</p>
        </div>
      </main>

      <aside className="transcript-panel" aria-label="Transcript">
        <h2 className="pane-heading">Transcript</h2>
        <p className="empty-hint">The live transcript appears here while you record.</p>
      </aside>
    </div>
  );
}
