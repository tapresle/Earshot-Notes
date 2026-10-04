import { useState } from "react";
import type { Meeting } from "../../lib/api";
import { groupByDay } from "./groupByDay";

interface MeetingListProps {
  meetings: Meeting[];
  selectedId: number | null;
  recordingMeetingId: number | null;
  onSelect: (id: number) => void;
  onCreate: () => void;
  creating: boolean;
}

const timeFormat: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };

export function MeetingList({
  meetings,
  selectedId,
  recordingMeetingId,
  onSelect,
  onCreate,
  creating,
}: MeetingListProps) {
  // Day labels are computed against the time the list mounted.
  const [now] = useState(() => Date.now());
  const groups = groupByDay(meetings, now);

  return (
    <nav className="sidebar" aria-label="Meetings">
      <button type="button" className="button primary full-width" onClick={onCreate} disabled={creating}>
        {creating ? "Creating…" : "New meeting"}
      </button>

      {groups.length === 0 ? (
        <p className="empty-hint sidebar-hint">Your meetings appear here.</p>
      ) : (
        groups.map((group) => (
          <section key={group.label} className="day-group" aria-label={group.label}>
            <h2 className="pane-heading">{group.label}</h2>
            <ul className="meeting-list">
              {group.meetings.map((meeting) => (
                <li key={meeting.id}>
                  <button
                    type="button"
                    className="meeting-item"
                    aria-current={meeting.id === selectedId ? "page" : undefined}
                    onClick={() => onSelect(meeting.id)}
                  >
                    <span className="meeting-title">{meeting.title}</span>
                    <span className="meeting-meta">
                      {meeting.id === recordingMeetingId
                        ? "Recording"
                        : new Date(meeting.createdAt).toLocaleTimeString(undefined, timeFormat)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        ))
      )}
    </nav>
  );
}
