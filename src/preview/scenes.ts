// The app states captured in docs/screenshots. Every name and line here is
// made up. Times are relative to the page's clock, which the screenshot run
// fixes, so day labels and durations never change between runs.
import type { MeetingDetail, TranscriptSegment } from "../lib/api";
import {
  chatExchange,
  createFakeBackend,
  savedSummary,
  segment,
  testModel,
  type FakeBackend,
} from "../testing/fakeBackend";

const MINUTE = 60_000;
const DAY = 24 * 60 * MINUTE;

function meeting(id: number, title: string, minutesAgo: number, lengthMinutes: number | null, notesMarkdown = ""): MeetingDetail {
  const startedAt = Date.now() - minutesAgo * MINUTE;
  return {
    id,
    title,
    createdAt: startedAt,
    startedAt,
    endedAt: lengthMinutes === null ? null : startedAt + lengthMinutes * MINUTE,
    notesMarkdown,
    notesUpdatedAt: startedAt,
  };
}

const DESIGN_NOTES = `- Onboarding copy is final once Megan signs off
- Move the empty state illustration to the left
- **Decision:** ship the new settings layout in 0.2

Follow up with Sam about the export folder picker.`;

const DESIGN_SUMMARY = `## Decisions

- The new settings layout ships in version 0.2.
- Onboarding copy is final after Megan's review on Thursday.

## Action items

- **Megan:** review the onboarding copy by Thursday.
- **Sam:** prototype the export folder picker.
- **You:** move the empty state illustration to the left.

## Discussion

The team compared two layouts for **Settings** and chose the grouped version, because it keeps model choices next to their sizes.`;

const DESIGN_TRANSCRIPT: [number, "me" | "them", string][] = [
  [4_000, "me", "Let's start with the onboarding copy."],
  [9_500, "them", "I have one more pass on the second screen, then it's final."],
  [16_000, "me", "Great. Can you have that done by Thursday?"],
  [20_500, "them", "Thursday works. I'll send the strings over when it's done."],
  [31_000, "me", "Next, the settings layout. Sam, did you compare the two versions?"],
  [37_500, "them", "The grouped one tested better. People found the model sizes faster."],
  [46_000, "me", "Then let's ship the grouped layout in 0.2."],
  [52_000, "them", "Sounds good. I'll prototype the export folder picker this week."],
];

const CALL_TRANSCRIPT: [number, "me" | "them", string][] = [
  [2_000, "them", "Thanks for making time today."],
  [6_500, "me", "Of course. What would you like to cover first?"],
  [11_000, "them", "Mostly the import flow. Our team keeps a lot of older recordings."],
  [18_500, "me", "Got it. Are those audio files or existing transcripts?"],
  [24_000, "them", "Transcripts, mostly in Markdown."],
];

function transcript(meetingId: number, lines: [number, "me" | "them", string][], firstId: number): TranscriptSegment[] {
  return lines.map(([startMs, source, text], i) => segment(firstId + i, meetingId, source, startMs, text));
}

/** The meetings shown in the sidebar, across several weeks: more than fit, so screenshots show the list scrolling. */
function history(): MeetingDetail[] {
  return [
    meeting(1, "Design sync", 90, 42, DESIGN_NOTES),
    meeting(2, "Launch planning", 5 * 60, 30, "- Launch moves to the second week of November"),
    meeting(3, "Weekly 1:1", DAY / MINUTE + 120, 25),
    meeting(4, "Hiring debrief", (3 * DAY) / MINUTE, 45),
    meeting(5, "Quarterly roadmap review", (8 * DAY) / MINUTE, 60),
    // Older meetings, so the list overflows and scrolls under New meeting and Settings.
    meeting(10, "Support handoff", (9 * DAY) / MINUTE, 20),
    meeting(11, "Pricing page review", (10 * DAY) / MINUTE + 90, 35),
    meeting(12, "Accessibility audit kickoff", (12 * DAY) / MINUTE, 50),
    meeting(13, "Weekly 1:1", (15 * DAY) / MINUTE + 120, 25),
    meeting(14, "Partner onboarding call", (16 * DAY) / MINUTE, 40),
    meeting(15, "Release retrospective", (17 * DAY) / MINUTE + 60, 55),
    meeting(16, "Search rewrite planning", (19 * DAY) / MINUTE, 45),
    meeting(17, "Customer interview", (22 * DAY) / MINUTE, 30),
  ];
}

/** A backend with the sample history, a summary model, Whisper chosen for transcripts, and the design sync's summary. */
function withHistory(extra: MeetingDetail[] = [], segments: TranscriptSegment[] = []): FakeBackend {
  const backend = createFakeBackend([...extra, ...history()], [...transcript(1, DESIGN_TRANSCRIPT, 1), ...segments]);
  const model = testModel("/Users/you/Library/Application Support/com.tedpresley.earshot/models/gemma-4-E2B-it-Q4_K_M.gguf");
  backend.state.installed = [{ ...model, source: "download" }];
  backend.state.model = backend.state.installed[0];
  backend.state.summaries.set(1, { ...savedSummary(1, DESIGN_SUMMARY), uptoMs: 42 * MINUTE });
  backend.state.whisperInstalled = true;
  backend.state.transcriptionModel = "whisper-large-v3-turbo-q5_0";
  return backend;
}

export const scenes: Record<string, () => FakeBackend> = {
  /** The first launch after setup: no meetings yet. */
  empty: () => createFakeBackend(),

  /** The setup checklist, before any permission is granted or model downloaded. */
  setup: () => {
    const backend = createFakeBackend();
    Object.assign(backend.state, {
      setupComplete: false,
      micPermission: "notDetermined",
      systemAudioPermission: "notDetermined",
    });
    return backend;
  },

  /** A finished meeting with notes and a transcript. */
  meeting: () => withHistory(),

  /** The design sync's summary. */
  summary: () => withHistory(),

  /** Questions about the design sync, answered with links to the transcript and notes. */
  chat: () => {
    const backend = withHistory();
    backend.state.chats.set(1, [
      ...chatExchange(
        1,
        1,
        "Which settings layout did we pick, and why?",
        "The grouped layout, because it tested better: people found the model sizes faster [S1]. It ships in 0.2 [S2, N1].",
        [
          { kind: "transcript", label: "S1", segmentId: 6, startMs: 37_500 },
          { kind: "transcript", label: "S2", segmentId: 7, startMs: 46_000 },
          { kind: "notes", label: "N1", text: "- **Decision:** ship the new settings layout in 0.2" },
        ],
      ),
      ...chatExchange(1, 3, "Who is sending the onboarding strings?", "Megan sends them on Thursday, after one more pass on the second screen [S1].", [
        { kind: "transcript", label: "S1", segmentId: 4, startMs: 20_500 },
      ]),
    ]);
    return backend;
  },

  /** A call in progress, transcribing both sides. */
  recording: () => {
    const call = meeting(6, "Customer call", 4, null, "- Import from Markdown transcripts\n- ");
    const backend = withHistory([call], transcript(6, CALL_TRANSCRIPT, 100));
    backend.state.recording = 6;
    return backend;
  },

  /** Settings, with Whisper and a summary model installed, and a vocabulary term. */
  settings: () => {
    const backend = withHistory();
    backend.state.vocabulary = [{ term: "Bluefin Ledger", heardAs: ["blue fin ledger", "bluefinn ledger"] }];
    return backend;
  },
};
