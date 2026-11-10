# Call graphs: measuring the panel and keeping per-render work to the screen

Part of the [herdr-review design](../../DESIGN.md). The mod is described in [gui](../gui.md), the stream it draws in
[gui-stream](gui-stream.md). What herdr-app was seen to do is in [herdr-app-facts](../herdr-app-facts.md).

A big review is slow because each render (a wheel tick, a hover) redoes work that grows with the review, not with the
screen. This file says what is measured, where the numbers go, and which per-render computations are kept.

## Measuring

Off unless the mod's props say `perf: true`. Off, the recorder is `noop()` (src/perf.ts): its calls do nothing and
`render` skips the item count, so a panel without the prop pays one empty call per render and per dispatch. The props are
read in `init` through `Host.props()`, not through `decodeProps`.

```
-> ReviewView.init
  -> recorderFor(host)                                 (pure)
       props is not an object, or props.perf is not true         E: the no-op recorder, same object every time
       else makeRecorder(clock, fileSink(host), 5 s)
-> ReviewView.render
  -> t0 = perf.start(); layout(model, now); draw(...)
       layoutCache calls the recorder's rebuild() on each miss, the call that runs streamLayout
       E: a hit calls nothing; a miss calls it once
  -> perf.render(t0, items drawn)                      items drawn = visibleRange end - first, 0 in the message view
-> ReviewView.dispatch
  -> t0 = perf.start(); update(...); the `scrolled` step; perf.dispatch(t0)
       the cmds, focus calls and notify after the step are not timed
-> the recorder, on any event
  -> the first event after the window (5 s) ended emits ONE line and clears the window; an idle panel writes nothing
       E: an event inside the window emits nothing
       E: an event after it emits one line, with that event in the window it starts
       E: flush() (close) emits what the window holds, and nothing when it holds nothing
  -> line: perf window=5.0s render n= mean= p95= max= | dispatch n= mean= p95= max= | layout_rebuilds= | drawn mean= max=
       E: percentile is the nearest rank, 0 for no values
       E: the line has count, mean, p95 and max for render and dispatch, the rebuilds, and the drawn items
```

**Where the line goes.** Nothing in herdr-app-facts proves a channel a mod can print to: `gpui-kit.d.ts` declares
`console`, but no spike saw where its output goes. What is proven is `writeTextFile(path, text, expected)` with
`appHome()` (sections 5 and 7: it creates the file, and the folder is already written to by the panel's settings). So the
line is appended to `${appHome}/data/herdr-review/perf.log` with the read-then-replace pattern of `savePanel`, and the
file keeps its last 200 lines. Read it with `tail -f ~/.config/herdr-app/data/herdr-review/perf.log`.

```
-> fileSink(host)(line)                                R: Host.readTextFile, Host.writeTextFile
  -> lines are written one after another (a promise chain), so two lines never race on `expected`
  -> read perf.log, append "<ISO time> <line>", keep the last 200 lines, write with the text read as `expected`
       E: the log keeps its last 200 lines
       E: the write goes to perf.log under the app home with the text it read as expected
       a refused write reads again, up to three times, then drops the line
       E: a sink whose reads fail drops the line and does not throw
```

The clock is `performance.now()` when the runtime has it, else `Date.now()`, which counts whole milliseconds: a render under
1 ms then reads as 0 or 1. Unknown: whether `performance` exists in the mod runtime.

## Work kept between renders

Every cache is keyed by the identity of the objects it reads (the model is immutable: a change makes a new object, and
`folds`, `files` and `state` are copied, never changed in place), held in a `WeakMap` keyed by one of them, so nothing is
shared between two copies of the mod and nothing outlives its input. What is drawn does not change.

| Cache (src/rows.ts) | Keyed by | Kept |
|---|---|---|
| `sidebarOf` behind `sidebarRows`, `filterCount`, `filterInfo` and `filterPick` | the `state.files` array, and the filter string (the latest one of each array) | the matches (`fileMatches` runs once per path), the rows, the count, the box's info |
| `totals` | the `State` | the sums |
| `unsentCount` | the `State` | the number |
| `noStops`, the `none` flag of `dock` | the `files` map, then the `State` and the `folds` map | whether the review has any stop; the answer of `stops(m).all.length === 0` without building the list or sorting threads |

A caller must not change what it gets back. A null state is `NO_FILES` / `NO_TOTALS`, so null does not miss every time.

```
-> sidebarRows(files, filter), filterCount(files, filter), filterInfo(m), filterPick(m)
       E: sidebar rows are what the uncached walk gives, and the same array while files and filter are the same
       E: a new filter or a new files array gives new rows
       E: the count and the box info agree with the uncached count and are kept
       E: Enter in the box picks from the same matches
-> totals(state)
       E: totals add the counts, and are the same object for the same state
-> unsentCount(state)
       E: unsent comments are counted as before and kept for the state
-> noStops(m)
       E: noStops is stops(m).all.length === 0 for a few models, and follows state, folds and files
```

Not kept, and still per render: `layoutCache` already keeps the stream (it is keyed by the model fields the stream reads);
`findComment` in `dock` walks every thread but only while a thread has the focus; `visibleRange` and `draw` of the items on
screen.

## Spike: a retained sidebar (for the user to run)

Not built. The question is whether the sidebar can be left out of a wheel tick's render altogether: the cache above makes
its rebuild cheap, but `sidebar()` still builds its elements for every file row on each render.

1. Run the panel with `perf: true` in the mod's props, open a review of a few hundred files, and scroll the diff for 10 s.
   Note `render` mean and p95 and `drawn` in the perf line. This is the baseline.
2. In a scratch copy of the mod, make the sidebar a retained child view: a `View` class of its own, made with
   `cx.new(...)` in `init`, that receives the rows, the filter state and the dispatch function, and draws what `sidebar()`
   draws now. `draw` puts it in the tree as a child instead of calling `sidebar()`.
3. Give the child only what the sidebar reads (the rows from `sidebarRows`, `filterInfo`, the panel side, `m.selected`, the
   hover) and call `notify` on it only when one of those changes, not on every model change.
4. Scroll the diff again for 10 s and read the perf line. If the sidebar is retained, a wheel tick should still render the
   parent (`render n` is the same) but should not rebuild the child. To see that, count the child's `render` calls in a
   second `perf` recorder, or log them: it should stay near zero during the scroll.
5. What to look at: does `render` mean drop against the baseline, by how much on a big review and by how much on a small
   one; does the hover highlight of a sidebar row and the selected file still follow the model (a stale child is the failure);
   does the filter box keep its focus when the parent re-renders; and does gpui-kit's `cx.new` exist for a child of a mod
   view at all (herdr-app-facts does not say, so check `gpui-kit.d.ts` first).
6. Keep it only if step 4 shows the saving; write what was seen in herdr-app-facts and turn the change into a bead.
