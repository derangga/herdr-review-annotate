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
  -> Msg.Loaded { key, state }
-> update(model, Loaded)                               (pure)
  -> inFlight = false
  -> key differs from the model's?                     E: stale reply -> dropped, the model not touched
     else state replaced, banner cleared, the spec the reply names taken
       a file that left the diff goes from `files`
       a loaded file whose summary changed goes into `dirty`: it is fetched again (a file on screen whose summary
       is the same is not, so it keeps its view)
       the selected file stays while it is in the diff, else it is the first
  -> pending? pending = false, inFlight = true, Cmd.Load       a change of key sets pending, so a stale
                                                       reply is followed by the load that was asked for
  -> else the files the stream wants? Cmd.LoadFiles    see gui-stream.md: one command at a time
  -> store watch not running and store_dir known? Cmd.WatchStore  R: Host.watchFolder, a loop in runner.watch
                                                       E: WatchUnavailable -> escape, timer only
-> update(model, Failed)                               (pure)
  -> inFlight = false; last good state and files kept
  -> banner or status line, by the E table in gui.md
  -> pending? one more load, except after TooOld, ContractMismatch and BinaryMissing, which stop the
     timer and watch loads until Refresh is clicked
```

A load is the state alone. The diffs of the files come after it, in the fetches of [gui-stream](gui-stream.md), so the file
list and the first screen show without waiting for every diff. A failure keeps the last good state and the diffs
already fetched.

The store directory may not exist yet: `state` creates nothing, and a repo nobody has commented on has no
store directory. The store watch then rejects, and the 10 s timer carries the panel until a load finds the
directory. A first comment from an agent shows within 10 s in that case.

Cardinality: many, one at a time.

## Switch the diff spec

```
-> SpecChosen { WorkTree | Branch { base } }           user action
  -> update: spec replaced, selected kept, every fetched diff dropped, Triggered
  -> load (above), with --spec and --base
       the CLI answers with the spec it used
       E: base missing -> the CLI falls back to the working tree and returns a notice. The model takes the
          spec the reply names, and shows the notice
```

The base list is not offered in v1. The branch spec uses the `base` the last `state` reply named, which is
the one in `meta.json` or the default base ([diff](../diff.md)). Another spec is another diff, so no file
fetched under the old one is kept: the stream shows each file's placeholder until its diff arrives.

## Choose a file

```
-> FileChosen { path }                                 user action
  -> update: selected = path, selection cleared, reveal = that file's header (gui-stream.md)
     no load of the state: the stream scrolls, and the files around the header are fetched as they show
```

A path that is not in the diff is ignored. An editor with text in it is not cleared by a file switch: the choice
is ignored with the status "finish or cancel the comment first".

## Tests

Each `E:` line is one test. Also: a burst of fifty triggers runs two loads at most, a trigger while the editor
is open runs none until it closes, a reply for the previous spec is dropped, a failed load keeps the screen,
a reply that leaves the selected file out of the diff selects the first file and drops its diff, and no `Host`
method other than `exec` and `watchFolder` is called.
