// GPUI drawing. It holds no logic and calls no Host method: everything it shows is in the model, the
// theme and the layout, and every click is a Msg handed to dispatch.
import { div, list, svg } from "gpui-kit";
import type { Element, FocusHandle } from "gpui-kit";
import { h_flex, Input, Scrollbar, Textarea, v_flex } from "gpui-base";
import type { InputState, TextareaState } from "gpui-base";
import { confirmKey, confirmRow, diffWidth, dock, editorKey, editorTitle, filterInfo, itemClick, listId, paneOrder, panelKey, pickerOptions, sidebarButtons, sidebarRows, totals } from "./rows.ts";
import type { Badge, BodyItem, DockButton, HeadItem, LayoutItem, RowItem, SideRow, Tone } from "./rows.ts";
import type { Model, Msg } from "./types.ts";

export type Theme = Record<string, `#${string}`>;
type Dispatch = (msg: Msg) => void;
// What the view owns and the model must not hold: the editor's text state, the focus handles that keys reach
// (`handle` for the question bar, `nav` for the whole panel), and the filter box's text state.
export type Ui = { handle: FocusHandle; nav: FocusHandle; textarea: TextareaState | null; filter: InputState };

const ROW = 20;
// The sidebar keeps this width and the diff takes the rest.
const SIDEBAR_W = 280;
// The diff scrolls sideways and is as wide as its longest line, so a card keeps to a width that reads.
const CARD_W = 560;
const tint = (color: `#${string}`, alpha: string): `#${string}` => `${color}${alpha}`;

// danger is a button that destroys something: the red of a removed line.
function button(id: string, label: string, on: boolean, t: Theme, click: (() => void) | null, danger = false): Element {
  return div()
    .id(id)
    .h(24)
    .px(8)
    .flex()
    .items_center()
    .rounded(6)
    .border_1()
    .border_color(danger ? t.ansi1 : on ? t.accent : t.border)
    .text_size(11)
    .text_color(on || danger ? t.foreground : t.muted)
    .when(on && !danger, (e) => e.bg(t.selected))
    .when(danger, (e) => e.bg(tint(t.ansi1, "30")))
    .when(click !== null, (e) => e.cursor_pointer().hover((s) => s.bg(t.hover)).on_click(() => click?.()))
    .when(click === null, (e) => e.opacity(0.4))
    .child(label);
}

const toneColor = (tone: Tone, t: Theme): `#${string}` => (tone === "user" ? t.blocked : tone === "agent" ? t.accent : t.ansi2);

function head(item: HeadItem, t: Theme): Element {
  const color = toneColor(item.tone, t);
  return div()
    .w_full()
    .flex_none()
    .pl(48)
    .pr(12)
    .pt(6)
    .child(
      v_flex()
        .max_w(CARD_W)
        .px(10)
        .py(4)
        .gap(2)
        .border_l(2)
        .border_color(color)
        .bg(item.focused ? t.selected : tint(color, "14"))
        .child(
          h_flex()
            .gap(8)
            .items_center()
            .when(item.collapsible, (e) => e.child(div().w(10).text_color(t.dim).child(item.open ? "\u25be" : "\u25b8")))
            .child(div().min_w(0).truncate().font_semibold().text_size(11).text_color(color).child(item.title))
            .children(item.badges.map((b) => div().flex_none().text_size(10).text_color(b === "unsent" || b === "new" ? t.blocked : t.dim).child(`[${b}]`))),
        )
        .when(item.quote !== null, (e) => e.child(div().truncate().text_size(11).text_color(t.dim).child(item.quote ?? ""))),
    );
}

function body(item: BodyItem, t: Theme): Element {
  const color = toneColor(item.tone, t);
  return div()
    .w_full()
    .flex_none()
    .pl(48)
    .pr(12)
    .when(item.last, (e) => e.pb(6))
    .child(
      v_flex()
        .max_w(CARD_W)
        .px(10)
        .py(2)
        .when(item.meta !== null, (e) => e.pl(22))
        .border_l(2)
        .border_color(color)
        .bg(item.focused ? t.selected : tint(color, "14"))
        .when(item.meta !== null, (e) => e.child(div().text_size(10).text_color(t.dim).child(item.meta ?? "")))
        .children(item.lines.map((line) => div().min_h(16).text_size(12).text_color(t.foreground).child(line === "" ? " " : line))),
    );
}

function item(i: LayoutItem, t: Theme): Element {
  if (i.kind === "head") return head(i, t);
  if (i.kind === "body") return body(i, t);
  if (i.kind === "more") return div().w_full().h(ROW).flex_none().px(12).flex().items_center().cursor_pointer().text_size(11).text_color(t.accent).child("\u2191 Show the start of the file");
  if (i.kind === "label") return div().w_full().flex_none().px(12).pt(8).pb(2).text_size(11).font_semibold().text_color(t.dim).child(i.text);
  return diffRow(i, t);
}

function diffRow(item: RowItem, t: Theme): Element {
  if (item.kind === "hunk") return div().w_full().px(12).h(ROW).flex_none().truncate().bg(t.raised).text_color(t.dim).child(item.text);
  const sign = item.kind === "add" ? "+" : item.kind === "del" ? "-" : " ";
  return h_flex()
    .w_full()
    .h(ROW)
    .flex_none()
    .gap(6)
    .when(item.kind === "add", (e) => e.bg(tint(t.ansi2, "18")))
    .when(item.kind === "del", (e) => e.bg(tint(t.ansi1, "18")))
    .when(item.selected, (e) => e.bg(tint(t.accent, "40")))
    .child(div().w(36).flex_none().text_right().text_color(t.dim).child(item.old))
    .child(div().w(36).flex_none().text_right().text_color(t.dim).child(item.new))
    .child(div().w(10).flex_none().text_color(item.kind === "add" ? t.ansi2 : item.kind === "del" ? t.ansi1 : t.dim).child(sign))
    .child(
      h_flex()
        .flex_1()
        .min_w(0)
        .children(item.runs.map((r) => div().flex_none().whitespace_nowrap().text_color(t[r.color] ?? t.foreground).child(r.text))),
    );
}

function dockBar(m: Model, t: Theme, dispatch: Dispatch, ui: Ui): Element {
  const e = m.editor;
  const wrap = v_flex().id("review-dock").flex_none().border_t(1).border_color(t.border).px(12).py(8).gap(6).text_size(11);
  if (e !== null && ui.textarea !== null) {
    const ta = ui.textarea;
    return wrap
      .track_focus(ui.handle)
      .on_key_down((ev, cx) => {
        const msg = editorKey(ev.keystroke, ta.value());
        if (msg !== null) {
          dispatch(msg);
          cx.stop_propagation();
        }
      })
      .child(div().font_semibold().text_color(t.blocked).child(editorTitle(e)))
      .child(Textarea.new(ta).w_full().px(8).py(6).rounded(6).border_1().border_color(t.accent).bg(t.background).text_size(12).text_color(t.foreground))
      .when(m.status !== null, (x) => x.child(div().text_color(t.ansi1).child(m.status ?? "")))
      .child(
        h_flex()
          .gap(8)
          .items_center()
          .child(button("editor-save", m.saving ? "Saving..." : "Save", true, t, () => dispatch({ _tag: "EditorSaved", text: ta.value() })))
          .child(button("editor-cancel", "Cancel", false, t, () => dispatch({ _tag: "EditorCancelled" })))
          .child(div().text_color(t.dim).child("Ctrl+S save, Esc cancel")),
      );
  }
  if (m.picker !== null) {
    return wrap
      .child(div().font_semibold().text_color(t.blocked).child("Several agents match this folder. Send to which?"))
      .children(pickerOptions(m.picker).map((b) => h_flex().child(button(`dock-${b.id}`, b.label, false, t, () => dispatch(b.msg)))));
  }
  const btn = (b: DockButton): Element => button(`dock-${b.id}`, b.label, false, t, b.off ? null : () => dispatch(b.msg), b.danger);
  const ask = confirmRow(m);
  if (ask !== null) {
    return wrap
      .track_focus(ui.handle)
      .on_key_down((ev, cx) => {
        const msg = confirmKey(ev.keystroke);
        if (msg !== null) {
          dispatch(msg);
          cx.stop_propagation();
        }
      })
      .child(div().font_semibold().text_color(t.blocked).child(ask.sentence))
      .child(h_flex().gap(8).items_center().children(ask.buttons.map(btn)));
  }
  const bar = dock(m);
  return wrap
    .child(
      h_flex()
        .gap(8)
        .items_center()
        .child(div().flex_1().min_w(0).truncate().text_color(t.muted).child(bar.label))
        .children(bar.buttons.map(btn)),
    )
    .child(h_flex().gap(8).items_center().flex_wrap().children(bar.actions.map(btn)));
}

const badgeColor = (b: Badge, t: Theme): `#${string}` => (b.tone === "unsent" ? t.blocked : b.tone === "open" ? t.accent : b.tone === "new" ? t.done : t.dim);

function sideRow(r: SideRow, m: Model, t: Theme, dispatch: Dispatch): Element {
  if (r.kind === "dir") return div().w_full().flex_none().px(8).pt(8).pb(2).truncate().text_ellipsis_start().text_size(11).text_color(t.dim).child(r.text);
  const letter = r.letter === "A" || r.letter === "?" ? t.ansi2 : r.letter === "D" ? t.ansi1 : t.blocked;
  return v_flex()
    .id(r.key)
    .w_full()
    .flex_none()
    .px(8)
    .py(3)
    .rounded(7)
    .cursor_pointer()
    .when(m.selected === r.path, (e) => e.bg(t.selected))
    .hover((s) => s.bg(t.hover))
    .on_click(() => dispatch({ _tag: "FileChosen", path: r.path }))
    .child(
      h_flex()
        .w_full()
        .gap(6)
        .items_center()
        .child(div().w(12).flex_none().text_size(11).text_color(letter).child(r.letter))
        .child(svg(`mods/herdr-review/icons/${r.icon}.svg`).size(14).flex_none().text_color(t.dim))
        .child(div().flex_1().min_w(0).truncate().text_color(r.viewed ? t.dim : t.foreground).child(r.name))
        .when(r.add > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi2).child(`+${r.add}`)))
        .when(r.del > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi1).child(`-${r.del}`))),
    )
    .when(r.badges.length > 0, (e) => e.child(h_flex().w_full().pl(38).gap(8).children(r.badges.map((b) => div().flex_none().text_size(10).text_color(badgeColor(b, t)).child(b.text)))));
}

// The files by folder in a column of their own, under the filter box. A folder heading is a plain div with no
// handler, so a click on it does nothing. The column has its own scroll handle and bar, laid over a relative box.
function sidebar(m: Model, t: Theme, dispatch: Dispatch, ui: Ui, empty: string): Element {
  const left = m.panel.side === "left";
  const rows = sidebarRows(m.state === null ? [] : m.state.files, m.filter);
  const info = filterInfo(m);
  return v_flex()
    .w(SIDEBAR_W)
    .flex_none()
    .h_full()
    .min_h(0)
    .when(left, (e) => e.border_r(1))
    .when(!left, (e) => e.border_l(1))
    .border_color(t.border)
    .child(
      div()
        .flex_none()
        .px(6)
        .pt(6)
        .child(
          h_flex()
            .id("review-filter")
            .h(28)
            .px(8)
            .gap(6)
            .items_center()
            .rounded(7)
            .border_1()
            .border_color(m.filtering ? t.accent : t.border)
            .cursor_text()
            // A press in the box is the box's own: it must not give the keyboard to the panel behind it.
            .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
            .child(div().flex_1().min_w(0).child(Input.new(ui.filter).h_full().text_size(12).text_color(t.foreground)))
            .child(div().flex_none().text_size(10).text_color(t.dim).child(info.count)),
        )
        .when(info.none, (e) => e.child(div().px(8).pt(4).text_size(11).text_color(t.dim).child("no match"))),
    )
    .child(
      div()
        .flex_1()
        .min_h(0)
        .relative()
        .child(
          v_flex()
            .id("review-files")
            .absolute()
            .inset_0()
            .overflow_y_scroll()
            .p(6)
            .children(rows.map((r) => sideRow(r, m, t, dispatch)))
            .when(rows.length === 0 && !info.none, (e) => e.child(div().p(8).text_color(t.dim).child(empty))),
        )
        .child(Scrollbar.vertical("review-files").absolute().inset_0()),
    );
}

// The diff of the selected file, and under it the bar that holds the selection, the thread buttons or the editor.
function diffPane(m: Model, items: LayoutItem[], t: Theme, dispatch: Dispatch, ui: Ui): Element {
  const width = diffWidth(items);
  const note = m.file !== null && m.file.note !== null ? m.file.note : null;
  return v_flex()
    .flex_1()
    .min_w(0)
    .h_full()
    .min_h(0)
    .child(
      div()
        .flex_1()
        .min_h(0)
        .relative()
        .text_size(11)
        .child(
          div()
            .id("review-diff-x")
            .size_full()
            .overflow_x_scrollbar()
            .child(
              div()
                .w_full()
                .min_w(width)
                .h_full()
                .child(
                  list(
                    listId(m.window),
                    items.length,
                    (i) => items[i]?.key ?? String(i),
                    (i) => item(items[i] ?? { kind: "label", key: "none", text: "" }, t),
                  )
                    .size_full()
                    .on_item_click((key) => {
                      const msg = itemClick(key);
                      if (msg !== null) dispatch(msg);
                    }),
                ),
            ),
        )
        .child(Scrollbar.vertical(listId(m.window)).viewport_from_layout().absolute().inset_0())
        .when(note !== null, (e) => e.child(div().absolute().top(6).px(12).text_color(t.dim).child(note ?? ""))),
    )
    .child(dockBar(m, t, dispatch, ui));
}

export function draw(m: Model, items: LayoutItem[], t: Theme, dispatch: Dispatch, ui: Ui): Element {
  const sum = totals(m.state);
  const base = m.state === null ? null : m.state.base;
  const notices = m.state === null ? [] : [...m.state.notices, ...m.state.warnings];
  const empty = m.banner !== null ? "" : m.state === null ? "Loading changes..." : "No changes";

  return v_flex()
    .id("review")
    .size_full()
    .min_h(0)
    .overflow_hidden()
    .bg(t.background)
    .text_color(t.foreground)
    .text_size(13)
    // Not while the editor is open: a click in its textarea must not give the keyboard to the panel.
    .when(m.editor === null, (e) =>
      e.track_focus(ui.nav).on_key_down((ev, cx) => {
        const msg = panelKey(m, ev.keystroke);
        if (msg !== null) {
          dispatch(msg);
          cx.stop_propagation();
        }
      }),
    )
    .child(
      h_flex()
        .min_h(42)
        .flex_none()
        .flex_wrap()
        .px(14)
        .py(6)
        .gap(8)
        .items_center()
        .border_b(1)
        .border_color(t.border)
        .child(div().font_semibold().child("Review"))
        .child(div().text_size(11).text_color(t.muted).child(`${sum.files} files`))
        .child(div().text_size(11).text_color(t.ansi2).child(`+${sum.add}`))
        .child(div().text_size(11).text_color(t.ansi1).child(`-${sum.del}`))
        .child(div().flex_1())
        .children(sidebarButtons(m).map((b) => button(b.id, b.label, false, t, () => dispatch(b.msg))))
        .child(button("spec-worktree", "Working tree", m.spec !== null && m.spec.kind === "worktree", t, () => dispatch({ _tag: "SpecChosen", spec: { kind: "worktree" } })))
        .child(
          button("spec-branch", base === null ? "Branch" : `Branch ${base}`, m.spec !== null && m.spec.kind === "branch", t, base === null ? null : () => dispatch({ _tag: "SpecChosen", spec: { kind: "branch", base } })),
        )
        .child(button("refresh", "Refresh", false, t, () => dispatch({ _tag: "Triggered", source: "refresh" }))),
    )
    .when(m.banner !== null, (e) => e.child(div().px(12).py(6).text_size(12).text_color(t.ansi1).child(m.banner ?? "")))
    .when(m.status !== null, (e) => e.child(div().px(12).py(4).text_size(11).text_color(t.blocked).child(m.status ?? "")))
    .when(notices.length > 0, (e) => e.child(div().px(12).py(4).text_size(11).text_color(t.dim).child(notices.join(" | "))))
    .child(
      h_flex()
        .flex_1()
        .min_h(0)
        .items_start()
        .children(paneOrder(m).map((p) => (p === "sidebar" ? sidebar(m, t, dispatch, ui, empty) : diffPane(m, items, t, dispatch, ui)))),
    );
}
