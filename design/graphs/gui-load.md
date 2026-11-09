# Call graphs: the GUI loads

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md). The two Rust
commands it calls are in [json](json.md).

## Load

```
-> trigger                                             many   (Msg.Triggered carries its source: watch, timer,
                                                       refresh, write, change or editor. Refresh also clears
                                                       `stopped`; a watch or timer trigger is ignored while
                                                       `stopped`)
     watch wake (root or store), the 10 s timer, Refresh, a write done, a root or spec or file change,
     the editor closing
  -> update(model, Triggered)                          (pure)
       inFlight or an editor open? pending = true, stop
       else inFlight = true, Cmd.Load { key }
-> run Cmd.Load                                        one in flight
  -> ReviewCli.state(root, spec)                       R: Host.exec, 60 s
                                                       E: BinaryMissing, TooOld, BackendFailed, BadRequest -> Msg.Failed
    -> decodeState(stdout)                             boundary
                                                       E: ContractMismatch -> Msg.Failed
  -> choose the file                                   (pure)
       the selected path if the state lists it, else the first file, else none
  -> ReviewCli.file(root, spec, path)                  R: Host.exec, 60 s     skipped when no file
                                                       E: as for state
                                                       E: BadRequest "not in diff" -> Msg.Failed, one reload
    -> decodeFile(stdout)                              boundary
                                                       E: ContractMismatch -> Msg.Failed
  -> Msg.Loaded { key, state, file }
-> update(model, Loaded)                               (pure)
  -> inFlight = false
  -> key differs from the model's?                     E: stale reply -> dropped, the model not touched
     else state, file and selected replaced, banner cleared
  -> pending? pending = false, inFlight = true, Cmd.Load       a change of key sets pending, so a stale
                                                       reply is followed by the load that was asked for
  -> store watch not running and store_dir known? Cmd.WatchStore  R: Host.watchFolder, a loop in runner.watch
                                                       E: WatchUnavailable -> escape, timer only
-> update(model, Failed)                               (pure)
  -> inFlight = false; last good state and file kept
  -> banner or status line, by the E table in gui.md
  -> pending? one more load, except after TooOld, ContractMismatch and BinaryMissing, which stop the
     timer and watch loads until Refresh is clicked
```

Both replies must arrive for `Loaded`. A failure of either keeps the whole last good model, so the file list
and the diff never come from two different moments.

The store directory may not exist yet: `state` creates nothing, and a repo nobody has commented on has no
store directory. The store watch then rejects, and the 10 s timer carries the panel until a load finds the
directory. A first comment from an agent shows within 10 s in that case.

Cardinality: many, one at a time.

## Switch the diff spec

```
-> SpecChosen { WorkTree | Branch { base } }           user action
  -> update: spec replaced, selected kept, Triggered
  -> load (above), with --spec and --base
       the CLI answers with the spec it used
       E: base missing -> the CLI falls back to the working tree and returns a notice. The model takes the
          spec the reply names, and shows the notice
```

The base list is not offered in v1. The branch spec uses the `base` the last `state` reply named, which is
the one in `meta.json` or the default base ([diff](../diff.md)).

## Choose a file

```
-> FileChosen { path }                                 user action
  -> update: selected = path, selection and editor cleared, Triggered
  -> load (above)
```

An editor with text in it is not cleared by a file switch: the choice is ignored with the status "finish or
cancel the comment first".

## Tests

Each `E:` line is one test. Also: a burst of fifty triggers runs two loads at most, a trigger while the editor
is open runs none until it closes, a reply for the previous spec is dropped, a failed load keeps the screen,
a path that left the diff selects the first file on the reload, and no `Host` method other than `exec` and
`watchFolder` is called.
