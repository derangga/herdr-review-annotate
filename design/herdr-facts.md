# What the design assumes about Herdr

Part of the [herdr-review design](../DESIGN.md).

Checked on Herdr 0.9.1 with the plugin linked from a checkout. `min_herdr_version` is 0.9.1. Raise it only
when a bug fixed in a later release is found.

- `--env` on `herdr plugin pane open` reaches the pane process. The review pane sees `REVIEW_DELIVER_TO`
  and `REVIEW_DELIVER_TERM`, its own `HERDR_PANE_ID`, and a `HERDR_PLUGIN_CONTEXT_JSON` whose
  `focused_pane_id` is the agent pane and whose `invocation_source` is `api`.
- The `send` action runs from the review pane, but its context has `focused_pane_id` set to the review pane,
  `focused_pane_status` `unknown` and no `focused_pane_agent`. From the review pane the action never sees
  the agent pane, so step 2 of target resolution in `send.md` (`meta.json`) is what finds it. Whether Herdr's command palette or
  key handler offers a `contexts = ["pane"]` action in the review pane is untested. `action invoke` is the
  only path the CLI exposes.
- `herdr config check` reports two custom commands on one key. It does not report a clash with a built-in
  key, and it does not check that the action named by a `plugin_action` exists. `prefix+r` and
  `prefix+shift+r` are built in, so the README suggests `prefix+i` and `prefix+shift+i`.
