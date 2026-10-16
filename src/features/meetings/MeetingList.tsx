import { type MouseEvent, type ReactNode, useCallback, useState } from "react";
import type { Meeting } from "../../lib/api";
import { ContextMenu, type MenuItem } from "./ContextMenu";
import { groupByDay } from "./groupByDay";
import type { SelectMode } from "./selection";
import { useToday } from "./useToday";

interface MeetingListProps {
  meetings: Meeting[];
  /** The open meeting. */
  selectedId: number | null;
  /** Meetings selected for a bulk action, including the open one. */
  selectedIds: number[];
  recordingMeetingId: number | null;
  onSelect: (id: number, mode: SelectMode) => void;
  /** Exports one meeting to a file, or several to a folder. */
  onExport: (ids: number[]) => void;
  /** Asks before deleting the meeting. */
  onDelete: (id: number, title: string) => void;
  onCreate: () => void;
  creating: boolean;
  /** Shown after the last meeting, scrolling with the list, such as Load more. */
  listEnd?: ReactNode;
  /** Kept at the bottom of the sidebar while the list scrolls. */
  footer?: ReactNode;
  /** Shown under the New meeting button, such as a search field. */
  search?: ReactNode;
  /** Shown when there are no meetings to list. */
  emptyHint?: string;
}

const timeFormat: Intl.DateTimeFormatOptions = { hour: "numeric", minute: "2-digit" };

export function MeetingList({
  meetings,
  selectedId,
  selectedIds,
  recordingMeetingId,
  onSelect,
  onExport,
  onDelete,
  onCreate,
  creating,
  listEnd,
  footer,
  search,
  emptyHint = "Your meetings appear here.",
}: MeetingListProps) {
  const multiSelect = selectedIds.length > 1;
  const today = useToday();
  const groups = groupByDay(meetings, today);
  const [menu, setMenu] = useState<{ meetingId: number; x: number; y: number; opener: HTMLElement } | null>(null);
  const closeMenu = useCallback(
    (restoreFocus: boolean) => {
      if (restoreFocus) menu?.opener.focus();
      setMenu(null);
    },
    [menu],
  );

  function openMenu(meetingId: number, event: MouseEvent<HTMLElement> | { currentTarget: HTMLElement }) {
    const opener = event.currentTarget;
    // From the keyboard, open under the meeting rather than at the pointer.
    const atPointer = "clientX" in event && (event.clientX !== 0 || event.clientY !== 0);
    const box = opener.getBoundingClientRect();
    setMenu({
      meetingId,
      x: atPointer ? event.clientX : box.left + 8,
      y: atPointer ? event.clientY : box.bottom,
      opener,
    });
  }

  /** Several meetings when the menu opens on one of a multi-selection, which can only be exported. */
  function menuFor(meetingId: number): { label: string; items: MenuItem[] } {
    const title = meetings.find((m) => m.id === meetingId)?.title ?? "Meeting";
    if (multiSelect && selectedIds.includes(meetingId)) {
      return {
        label: `${selectedIds.length} selected meetings`,
        items: [{ label: `Export ${selectedIds.length} meetings`, onSelect: () => onExport(selectedIds) }],
      };
    }
    return {
      label: title,
      items: [
        { label: "Export", onSelect: () => onExport([meetingId]) },
        { label: "Delete", onSelect: () => onDelete(meetingId, title), danger: true },
      ],
    };
  }

  return (
    <nav className="sidebar" aria-label="Meetings">
      <div className="sidebar-top">
        <button type="button" className="button primary full-width" onClick={onCreate} disabled={creating}>
          {creating ? "Creating…" : "New meeting"}
        </button>
        {search}
      </div>

      {/* Only the list scrolls; New meeting, search, and the footer stay in place. */}
      <div className="sidebar-list">
      {groups.length === 0 ? (
        <p className="empty-hint sidebar-hint">{emptyHint}</p>
      ) : (
        groups.map((group) => (
          <section key={group.label} className="day-group" aria-label={group.label}>
            <h2 className="pane-heading">{group.label}</h2>
            <ul className="meeting-list">
              {group.meetings.map((meeting) => {
                const meta =
                  meeting.id === recordingMeetingId
                    ? "Recording"
                    : new Date(meeting.createdAt).toLocaleTimeString(undefined, timeFormat);
                const inSelection = multiSelect && selectedIds.includes(meeting.id);
                return (
                <li key={meeting.id}>
                  <button
                    type="button"
                    className={`meeting-item ${inSelection ? "is-selected" : ""}`}
                    aria-label={`${meeting.title}, ${meta}${inSelection ? ", selected" : ""}`}
                    aria-current={meeting.id === selectedId ? "page" : undefined}
                    onClick={(event) =>
                      onSelect(
                        meeting.id,
                        event.shiftKey ? "range" : event.metaKey || event.ctrlKey ? "toggle" : "single",
                      )
                    }
                    onContextMenu={(event) => {
                      event.preventDefault();
                      openMenu(meeting.id, event);
                    }}
                    onKeyDown={(event) => {
                      if (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10")) {
                        event.preventDefault();
                        openMenu(meeting.id, event);
                      }
                    }}
                  >
                    <span className="meeting-title">{meeting.title}</span>
                    <span className="meeting-meta">{meta}</span>
                  </button>
                </li>
                );
              })}
            </ul>
          </section>
        ))
      )}
      {listEnd}
      </div>
      {footer && <div className="sidebar-bottom">{footer}</div>}
      {menu && <ContextMenu {...menuFor(menu.meetingId)} x={menu.x} y={menu.y} onClose={closeMenu} />}
    </nav>
  );
}
