# Model switcher: pick a dot's model from a list, set a default, switch it from the chat

- Date: 2026-11-05
- Status: approved (owner's request, 2026-11-05)
- Builds on: the dot editor (`DotForm`), the local engine (`engine/local`), Settings.

## 1. Goal

"I want a model switcher so I can switch models from Sonnet to Opus or any model I want to run,
directly choose it in Dots rather than having just one default model running always."

Today every dot has a free-text `DotSpec.model` (default `sonnet`), typed into a box in the dot
editor. Nothing lists the models there are, there is no default other than `sonnet`, and the
model can only be changed by opening the editor.

## 2. Behaviour

1. **Catalog.** The core lists the models a dot can use, by engine (`list_models`):
   - **Claude**: one curated list, kept in one place (`dots_core::models::CLAUDE_MODELS`): Opus
     (`opus`, most capable, slower), Sonnet (`sonnet`, balanced, the default) and Haiku
     (`haiku`, fastest and cheapest). The ids are the CLI's aliases, passed as `--model`.
   - **Local**: the ids the endpoint lists at `GET {endpoint}/v1/models` (OpenAI-compatible:
     `{"data": [{"id": …}]}`). The endpoint is checked as the "Test endpoint" probe checks it
     (`check_endpoint_url`: plain `http://` with a host). An endpoint that is unreachable, answers
     with an error or with something that is not a model list gives an error that says so; the
     UI shows it and still offers a custom model id.
2. **Default model** (Settings, Claude Code CLI section): `default_model`, the model a new
   Claude dot starts with; `sonnet` when unset, so an old settings table loads as before. It
   must not be empty. New dots made from the UI start with it: the dot editor's blank dot
   (`new_dot_spec`), a dot the setup agent makes, and an imported agent that names no model.
   Existing dots keep their model.
3. **Picker in the dot editor.** The free-text box becomes a select of the catalog (for a local
   dot, the endpoint's list, loaded when an endpoint is set) and **Custom…**, which shows a text
   box for any id (a full id such as `claude-opus-4-1`, or a local model the endpoint does not
   list). A dot whose model is not in the list shows as Custom with its id. The editor's check
   (a model must be named) is unchanged; the core's is too.
4. **Quick switcher in the conversation.** The header of a dot's conversation shows a compact
   model select with the dot's current model. Changing it saves the dot with the new model
   through `update_dot`, the same command the editor saves with, so every rule that applies to
   editing a dot (the role's `EditDots` permission, a removed org) applies; a refusal is shown
   and the select goes back. The change applies to the next reply; a run already working keeps
   the model it started with. The select says so in its tooltip and label.
5. **Skill creator.** "Create skill" picks its model with the same component (Claude catalog plus
   Custom…).

## 3. Non-goals

- No per-message model override, and no model column on runs: the model is the dot's.
- No new providers and no API keys for hosted APIs: the engines stay the `claude` CLI and a
  local OpenAI-compatible endpoint.
- No migration: `dots.model` is unchanged.

## 4. Changes

- Core: `models.rs` (`ModelOption`, `CLAUDE_MODELS`, `list_models`), `engine/local`
  (`models_url`, `list_models`), `settings.rs` (`default_model`), `runtime.rs`
  (`new_dot_spec`, the setup agent's and imported agents' dots).
- App: commands `list_models` and `new_dot_spec` (now reads Settings); the contract samples
  `ModelOption`.
- UI: `components/ModelPicker.tsx` (select + Custom…), used by `DotForm`, Settings,
  `CreateSkillDialog` and the conversation header (`DotModelSwitch`).
