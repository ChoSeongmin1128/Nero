# Local terminology correction

Terminology correction follows ordinary recognition. The recognizer receives no global
dictionary prompt. A run first saves its raw output and initial transcript, then may
create a separate correction revision. Existing recordings are not processed on startup.

## Local profile

An optional `terminology/active.json` in the application's own data directory contains
`schemaVersion: 1`, `enabled` and a `profile`. Profiles contain a stable ID, revision and
explicit rules with canonical spelling, configured source forms, kind, positive and
exclusion context cues, and an optional definition. Organization terms and profiles are private user data, never
bundled defaults. Missing profiles hide the Recording setting; ordinary status reads
do not create files. The Recording setting toggles the local profile for future runs.

- `acronym`: only joins the same uppercase ASCII letters separated by spaces. A match
  inside a longer acronym is retained unchanged.
- `contextual`: requires an exact configured source form, an independent literal cue
  in the same or adjacent utterance and an explicit contextual-model acceptance.
  An exclusion cue in that window blocks replacement regardless of the model's decision.
  Definitions explain a term; they cannot introduce facts into a transcript.

Candidates are exact UTF-8 spans bound to the source and profile hashes. Arbitrary
rewrites, embedded-word expansions, overlapping candidates and existing canonical
spellings are excluded. Particles outside a matched span remain unchanged; a change
that would invalidate a final-consonant-dependent particle is skipped. These
constraints limit edits; they are not a guarantee of recognition or contextual accuracy.

## Optional contextual model

If the separately prepared summary model is available, its existing local runtime
classifies fixed candidates as accept, keep or uncertain. It cannot generate replacement
wording. Accepted decisions must cite an actual supplied utterance containing an
independent cue. Missing, malformed, uncertain or out-of-context decisions retain the
recognition text. Without that model, acronym formatting still works and contextual
names remain unchanged. This feature does not download or require the optional model.

The ASR worker finishes before contextual inference starts. Both stages share the
processing job and cancellation controls. The contextual stage reuses the existing
authenticated local runtime, bounded context, GPU-first recovery and worker cleanup.
No Slack, Notion or remote model request occurs during correction.

## Persistence and conflicts

The profile is frozen at the beginning of recognition. Before applying corrections,
the core compares the complete source digest and current transcript version. A concurrent
user edit wins; an automatic result never overwrites it. Only a new initial recognition
revision is eligible. Utterance IDs, order, original text, times, source indices and
speaker assignments are retained. Corrections use the existing transcript revision writer.

The immutable correction receipt records the profile, candidates, decisions, original
source, hashes and resulting revision before replacing the current pointer. Job evidence
also records skipped and unsuccessful decisions. A failed evidence write preserves the
saved baseline. Cancellation accepted before final commit prevents correction; final
storage rejects a late cancellation. Previous recognition runs remain available through
the existing Previous records workflow; automatic correction does not delete raw or initial files.

Validation distinguishes deterministic span/storage tests, mocked protocol tests and
actual local-model runs. None establish general Korean accuracy, speaker identity or
Windows runtime qualification. Platform packaging and runtime support must be verified independently.
