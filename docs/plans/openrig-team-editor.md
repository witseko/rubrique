# OpenRig team editor

Status: design. Mockups: https://claude.ai/artifact/TtwmjwWCwTYnQReAka27QA

## The problem

Rubrique shows the seats of a running OpenRig team and sends tasks to them,
but it cannot create or change a team. A team is defined in a rig spec
(`rig.yaml`): pods, the seats in each pod, and the links between seats.
Today people write that YAML by hand. OpenRig's own web UI and TUI only show
a spec or take raw YAML.

## The editor

A rig spec opens as a form in a document tab. A Form / YAML switch at the
top right shows the same file as text. Nothing else in the app changes
place: the Explorer OpenRig section and Settings stay as they are.

1. **Header.** Team name, a count line ("3 pods, 7 seats, 11 links"), the
   summary, and Validate and Launch buttons.
2. **Seats.** One card per pod (label and id). One row per seat: id, runtime
   pill, model, agent, profile. A line under the rows lists the links inside
   the pod. "Add seat" on each card, "Add pod" below the cards.
3. **Seat inspector.** Opens inside the tab, on the right, for the selected
   seat.
   - Basic: seat id with the live address (`dev-build@starter`), agent,
     profile, runs on (pills), model, effort, folder, role, permissions.
   - Advanced (closed): startup files, environment, restore policy,
     compaction, then the Claude Code only or Codex only fields for the
     chosen runtime.
   - Skills are read-only. They belong to the agent's profile, so the
     inspector shows them with an "Open agent.yaml" link.
   - A terminal seat fixes agent to `builtin:terminal` and profile to `none`,
     and disables role and permissions.
4. **Who works with whom.** The links as rows ("build delegates to review")
   with a kind drop-down, for links inside a pod and between pods.
5. **Team.** Name, summary, permissions, workspace and repos, environment,
   culture file. Advanced (closed): docs, startup, managed blocks,
   non-interruptive, services.

States in the mockups: new team (from starter, from factory, blank), a
built-in team shown read-only with "Duplicate to edit", validation errors on
the fields, the preflight result, `rig` not installed, unsaved changes, and
fields the form does not cover.

## Field mapping

The source of truth is `docs/reference/rig-spec.md` and
`packages/daemon/src/domain/rigspec-schema.ts` in the OpenRig repository.

| Form label | Spec key | Values |
| --- | --- | --- |
| Seat id | `id` | no `.` or `@`, unique in the pod |
| Agent | `agent_ref` | `local:<relative path>`, `path:<absolute path>`, `builtin:terminal` |
| Profile | `profile` | a profile of the agent; `none` for a terminal seat |
| Runs on | `runtime` | `claude-code`, `codex`, `pi`, `omp`, `terminal` |
| Model, Effort | `model`, `effort` | free text; empty means the runtime default |
| Folder | `cwd` | relative to the spec's folder |
| Role | `role` | letters, digits, `_`, `.`, `-` |
| Permissions | `permission_policy` | `builtin:locked`, `standard`, `open`, `yolo`, `auto`, or a policy file |
| After a restart | `restore_policy` | `resume_if_possible`, `relaunch_fresh`, `checkpoint_only` |
| Compaction | `compaction_strategy` | `default-compaction`, `managed-compaction`, `handover`, `apprentice-handover` |
| Link kind | `edges[].kind` | `delegates_to`, `spawned_by`, `can_observe`, `collaborates_with`, `escalates_to` |

Kept as written and edited in YAML only: `claude_auto_mode`,
`launch_wrapper`, `process_limits`, `session_source`, `starter_ref`, startup
actions for agent seats, and the pod's `continuity_policy` and `startup`.
The `stub` runtime and the `none` permission value are not offered.

## Open questions

- **Saving.** The form promises that YAML comments and fields it does not
  cover survive a save. That needs a YAML writer that keeps comments.
- **Duplicating a built-in team.** Built-in teams point to agents with
  relative `local:` paths that do not resolve from `~/.openrig/specs/`. The
  mockup rewrites them to `path:` links on duplicate.
- **Pod id and label.** The mockups show them read-only; the edit control is
  not designed yet.

## Implementation notes

- A new `OpenFile` variant (`src/app.rs`) for the form view of a rig spec,
  with the YAML view reusing the text editor.
- Validate runs `rig spec validate <path> --json` (no daemon). Its error
  paths (`pods[0].members[1].id`) map to form fields. Preflight runs
  `rig spec preflight <path> --json`. Launch runs `rig up <path>`, with
  `--plan` for a preview. These go through `Rig` in `src/openrig.rs`, so
  remote rigs over SSH keep working.
- User specs live in `~/.openrig/specs`; built-in specs are read-only.
- Reuse the Settings widgets (`list_frame`, `tag`, `field_box`,
  `choice_chip`) and the constants of `src/settings/view/openrig_page.rs`.
