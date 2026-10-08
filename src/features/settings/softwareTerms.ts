/**
 * Technical terms that transcription turns into everyday words, such as
 * "async" as "a sink" or "params" as "palms", offered as a one-step addition
 * to the vocabulary (#104). Terms only, never mishearings: a rule such as
 * "a sink" to "async" would be wrong outside software meetings.
 *
 * Measured on a real call and on non-software speech before shipping.
 * Common words that transcribe well, such as "commit" and "branch", are left
 * out because they gain nothing and risk false hits. "Docker" turned
 * "document" into "docker", and "undefined" garbled an unrelated sentence, so
 * neither is listed.
 */
export const SOFTWARE_TERMS: readonly string[] = [
  "async",
  "await",
  "nullable",
  "null",
  "boolean",
  "enum",
  "JSON",
  "YAML",
  "API",
  "endpoint",
  "payload",
  "query params",
  "params",
  "webhook",
  "OAuth",
  "localhost",
  "SQL",
  "regex",
  "schema",
  "middleware",
  "cron",
  "git",
  "GitHub",
  "npm",
  "TypeScript",
  "JavaScript",
  "Kubernetes",
  "refactor",
];

/** The terms in `preset` not already in `existing`, ignoring case and surrounding spaces. */
export function missingTerms(existing: readonly string[], preset: readonly string[]): string[] {
  const have = new Set(existing.map((term) => term.trim().toLowerCase()));
  return preset.filter((term) => !have.has(term.toLowerCase()));
}
