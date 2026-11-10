// The exported View: init, render, close, and the loops. The model lives on the instance, never in module
// scope, because every copy of the mod shares the module.
import { View } from "gpui-kit";
import type { AsyncContext, Context, Element, FocusHandle } from "gpui-kit";
import { InputState, TextareaState } from "gpui-base";
import { decodeProps } from "./decode.ts";
import { makeCli } from "./cli.ts";
import { realHost } from "./host.ts";
import { listen, open, run, watch } from "./runner.ts";
import type { Env } from "./runner.ts";
import { layoutCache, scrolled } from "./rows.ts";
import { initModel, update } from "./update.ts";
import type { Cmd, Model, Msg } from "./types.ts";
import { draw } from "./view.ts";

const TIMER_MS = 10_000;
// A card has been looked at once it is still on screen a whole tick later, so a fast scroll marks nothing.
const GLANCE_MS = 500;

export default class ReviewView extends View {
  model: Model = initModel();
  closed = false;
  cx!: AsyncContext;
  env!: Env;
  handle!: FocusHandle;
  nav!: FocusHandle;
  textarea: TextareaState | null = null;
  filter!: InputState;
  // The stream's items, heights and prefix sums, kept while nothing they are built from changes (a wheel tick, a hover and a
  // drag change none of it).
  layout = layoutCache();

  init(_props: unknown, cx: AsyncContext): void {
    this.cx = cx;
    this.handle = cx.focus_handle();
    this.nav = cx.focus_handle();
    // The filter box. Its text is mirrored into the model on every change, and focus and blur say whether a key
    // types or is a command.
    this.filter = InputState.new({ placeholder: "filter (/)" });
    this.filter.on("change", () => this.dispatch({ _tag: "FilterChanged", text: this.filter.value() }));
    this.filter.on("submit", () => this.dispatch({ _tag: "FilterEntered" }));
    this.filter.on("focus", () => this.dispatch({ _tag: "FilterFocused", on: true }));
    this.filter.on("blur", () => this.dispatch({ _tag: "FilterFocused", on: false }));
    this.env = {
      host: realHost,
      cli: makeCli(realHost, decodeProps(realHost.props()).bin),
      dispatch: (msg) => this.dispatch(msg),
      closed: () => this.closed,
    };
    const opened = open(realHost);
    this.dispatch(opened);
    this.start({ _tag: "ReadPanel" });
    cx.spawn(() => listen(this.env, () => this.focusNav()));
    if (opened._tag === "Opened" && opened.root !== null) {
      const root = opened.root;
      cx.spawn(() => watch(this.env, root, "watch-root"));
      cx.timer.every(TIMER_MS, () => this.dispatch({ _tag: "Triggered", source: "timer" }));
      cx.timer.every(GLANCE_MS, () => this.dispatch({ _tag: "Glanced" }));
    }
  }

  dispatch(msg: Msg): void {
    if (this.closed) return;
    const before = this.model;
    let [model, cmds] = update(before, msg);
    // The offset, the stream's height and the files on screen follow the stream whatever moved it: a wheel tick, the
    // thumb, a file that arrived, a request to scroll to an item. One more step makes the model say so.
    if (model !== before) {
      const sync = scrolled(model, this.layout(model, Date.now()));
      if (sync !== null) {
        const [next, more] = update(model, sync);
        model = next;
        cmds = [...cmds, ...more];
      }
    }
    this.model = model;
    if (model.editor !== before.editor) this.syncEditor();
    if ((model.confirm !== null && model.confirm !== before.confirm) || (model.confirmArchive && !before.confirmArchive)) this.focusAsk();
    // The panel takes the keyboard back once a jump is asked, and once the editor or a question is closed.
    if (msg._tag === "ThreadJumped" || (before.editor !== null && model.editor === null) || ((before.confirm !== null || before.confirmArchive) && model.confirm === null && !model.confirmArchive)) this.focusNav();
    // A press on a line, and the keys (Esc) go to the panel only when it holds the keyboard.
    if (msg._tag === "RowPressed") this.focusNav();
    // Esc is the one change the box did not make itself: it empties the box.
    if (msg._tag === "FilterCleared") this.filter.set_value("");
    if (msg._tag === "FilterOpened") this.focusFilter();
    // The model gave the keyboard back (Enter on a match, Esc), which is not a click elsewhere (FilterFocused).
    if (before.filtering && !model.filtering && msg._tag !== "FilterFocused") this.focusNav();
    if (model !== before) this.cx.notify();
    for (const cmd of cmds) this.start(cmd);
  }

  // The editor's text lives in a textarea state the view owns: made when an editor opens, released when it
  // closes. Its box in the stream has a fixed height (EDITOR_TEXT), so it does not grow with the text. Dispatch runs in click handlers when an editor opens, where creating state is allowed.
  syncEditor(): void {
    this.textarea?.release();
    this.textarea = null;
    const e = this.model.editor;
    if (e === null) return;
    const ta = TextareaState.new({ placeholder: "Write a note...", value: e.initial, rows: 5 });
    ta.focus();
    this.cx.timer.after(600, () => {
      if (this.textarea === ta) ta.focus();
    });
    this.textarea = ta;
  }

  // A question is up: the keyboard goes to the bar that holds it, so Esc reaches it. Tried again once the
  // bar is drawn, as for the editor.
  focusAsk(): void {
    this.handle.focus();
    this.cx.timer.after(300, () => {
      if (this.model.confirm !== null || this.model.confirmArchive) this.handle.focus();
    });
  }

  // The keyboard goes to the panel so n and N reach it. Tried again once the root is drawn with its handle,
  // as for the editor.
  focusNav(): void {
    this.nav.focus();
    this.cx.timer.after(300, () => {
      if (!this.closed && this.model.editor === null) this.nav.focus();
    });
  }

  // `/`: the keyboard goes to the box. Tried again once it is drawn, for a sidebar that was hidden.
  focusFilter(): void {
    this.filter.focus();
    this.cx.timer.after(300, () => {
      if (!this.closed && this.model.panel.open && !this.model.filtering) this.filter.focus();
    });
  }

  start(cmd: Cmd): void {
    this.cx.spawn(async () => {
      const reply = await run(this.env, cmd);
      if (reply !== null) this.dispatch(reply);
    });
  }

  render(_cx: Context): Element {
    return draw(this.model, this.layout(this.model, Date.now()), realHost.theme(), (msg) => this.dispatch(msg), { handle: this.handle, nav: this.nav, textarea: this.textarea, filter: this.filter });
  }

  // Bounded: set the flag and return. The watch loops end after their next wake.
  close(_cx: AsyncContext): void {
    this.closed = true;
    this.textarea?.release();
    this.textarea = null;
    this.filter.release();
  }
}
