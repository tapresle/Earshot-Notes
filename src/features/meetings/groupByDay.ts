import type { Meeting } from "../../lib/api";

export interface DayGroup {
  label: string;
  meetings: Meeting[];
}

/** The start of the local day containing `ms`. */
export function startOfDay(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

function dayLabel(dayStart: number, now: number, locale?: string): string {
  const today = startOfDay(now);
  if (dayStart === today) return "Today";
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);
  if (dayStart === yesterday.getTime()) return "Yesterday";

  const sameYear = new Date(dayStart).getFullYear() === new Date(now).getFullYear();
  return new Date(dayStart).toLocaleDateString(locale, {
    weekday: "short",
    month: "short",
    day: "numeric",
    ...(sameYear ? {} : { year: "numeric" }),
  });
}

/** Groups meetings, already sorted newest first, by the local day they were created. */
export function groupByDay(meetings: Meeting[], now: number, locale?: string): DayGroup[] {
  const groups: DayGroup[] = [];
  let currentDay: number | null = null;
  for (const meeting of meetings) {
    const day = startOfDay(meeting.createdAt);
    if (day !== currentDay) {
      groups.push({ label: dayLabel(day, now, locale), meetings: [] });
      currentDay = day;
    }
    groups[groups.length - 1].meetings.push(meeting);
  }
  return groups;
}
