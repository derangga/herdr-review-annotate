// GPUI drawing. It holds no logic and calls no Host method: everything it shows is in the model, the
// theme and the layout, and every click is a Msg handed to dispatch.
import { div, svg } from "gpui-kit";
import type { Context, Element, FocusHandle, ScrollWheelEvent } from "gpui-kit";
import { h_flex, Input, Scrollbar, Textarea, v_flex } from "gpui-base";
import type { InputState, TextareaState } from "gpui-base";
import { badgeColor, CARD_W, codeWidth, confirmKey, confirmRow, cursorOn, dock, EDITOR_H, EDITOR_TEXT, editorKey, filterInfo, GUTTER_W, isSplit, itemClick, keyPath, layoutButton, letterColor, maxShift, NO_FILES, paneOrder, panelKey, pickerOptions, PLUS, rowState, ROW, sidebarButtons, sidebarRows, tagColor, thumbOf, toneColor, totals, viewedClick, visibleRange, wheelDelta, wheelShift, WARN } from "./rows.ts";
import type { BodyItem, DockButton, EditorItem, FileItem, HeadItem, RowItem, SideRow, Side, SplitItem, Stream, StreamItem } from "./rows.ts";
import type { Model, Msg } from "./types.ts";
import { drawMessage } from "./message-view.ts";

export type Theme = Record<string, `#${string}`>;
type Dispatch = (msg: Msg) => void;
// What the view owns and the model must not hold: the editor's text state, the focus handles that keys reach
// (`handle` for the question bar, `nav` for the whole panel) and the filter box's text state.
export type Ui = { handle: FocusHandle; nav: FocusHandle; textarea: TextareaState | null; filter: InputState };

// The sidebar keeps this width and the diff takes the rest.
const SIDEBAR_W = 280;
const tint = (color: `#${string}`, alpha: string): `#${string}` => `${color}${alpha}`;
const CLEAR = "#00000000";

const drawing = (name: string): string => `mods/herdr-review/icons/${name}.svg`;

// danger is a button that destroys something: the red of a removed line. `icon` puts a drawing before the label and `tip` is a
// tooltip, for a button that keeps its words but has more to say (a count, why it is off).
export function button(id: string, label: string, on: boolean, t: Theme, click: (() => void) | null, danger = false, icon?: string, tip?: string): Element {
  return div()
    .id(id)
    .h(24)
    .px(8)
    .flex()
    .items_center()
    .gap(6)
    .rounded(6)
    .border_1()
    .border_color(danger ? t.ansi1 : on ? t.accent : t.border)
    .text_size(11)
    .text_color(on || danger ? t.foreground : t.muted)
    .when(on && !danger, (e) => e.bg(t.selected))
    .when(danger, (e) => e.bg(tint(t.ansi1, "30")))
    .when(tip !== undefined, (e) => e.tooltip(tip ?? "").accessibility_label(tip ?? ""))
    // The press is the button's own: a button on a card must not also fire the card's click behind it.
    .when(click !== null, (e) => e.cursor_pointer().hover((s) => s.bg(t.hover)).on_click((_e, cx) => {
      cx.stop_propagation();
      click?.();
    }))
    .when(click === null, (e) => e.opacity(0.4))
    .when(icon !== undefined, (e) => e.child(svg(drawing(icon ?? "")).size(14).flex_none().text_color(danger ? t.ansi1 : on ? t.accent : t.muted)))
    .child(label);
}

// A toggle, a step or a repeat action, as a drawing alone: `tip` is its tooltip and its name for a screen reader, so it is required
// (and says why when the button is off). `on` is a toggle that is lit, `danger` a button that destroys something. A square of `size`.
export type IconState = "rest" | "on" | "danger";
export function iconButton(id: string, icon: string, tip: string, state: IconState, t: Theme, click: (() => void) | null, size = 28): Element {
  const color = state === "danger" ? t.ansi1 : state === "on" ? t.accent : t.muted;
  return div()
    .id(id)
    .w(size)
    .h(size)
    .flex_none()
    .flex()
    .items_center()
    .justify_center()
    .rounded(6)
    .text_color(color)
    .when(state === "on", (e) => e.bg(t.selected))
    .when(state === "danger", (e) => e.bg(tint(t.ansi1, "30")))
    .tooltip(tip)
    .accessibility_label(tip)
    // As in `button`, the press is the button's own.
    .when(click !== null, (e) => e.cursor_pointer().hover((s) => s.bg(t.hover).text_color(state === "rest" ? t.foreground : color)).active((s) => s.bg(t.selected)).on_click((_e, cx) => {
      cx.stop_propagation();
      click?.();
    }))
    .when(click === null, (e) => e.opacity(0.4))
    .child(svg(drawing(icon)).size(16).flex_none().text_color(color));
}

// One of a record's buttons (rows.ts `DockButton`): an icon alone, or its words with the icon before them. The tooltip is `tip` or
// the label, with the key that does the same in brackets.
export function recordButton(id: string, b: DockButton, t: Theme, dispatch: Dispatch, size = 24): Element {
  const click = b.off ? null : () => dispatch(b.msg);
  // A `tip` is a reason the button is off or a count, so the key is not added to it.
  const tip = b.tip !== undefined || b.key === undefined ? (b.tip ?? b.label) : `${b.label} (${b.key})`;
  if (b.icon !== undefined && b.iconOnly === true) return iconButton(id, b.icon, tip, b.danger ? "danger" : b.accent === true ? "on" : "rest", t, click, size);
  return button(id, b.label, b.accent === true, t, click, b.danger, b.icon, b.icon === undefined ? undefined : tip);
}

// A choice of one among a few words in a single track. The chosen segment is lit; one with no `click` is off, and its tooltip says why.
export type Segment = { id: string; label: string; tip: string; on: boolean; click: (() => void) | null };
export function segmented(id: string, options: Segment[], t: Theme): Element {
  return h_flex()
    .id(id)
    .h(28)
    .flex_none()
    .p(2)
    .gap(2)
    .items_center()
    .rounded(7)
    .border_1()
    .border_color(t.border)
    .children(
      options.map((o) =>
        div()
          .id(o.id)
          .h(22)
          .px(10)
          .flex()
          .items_center()
          .rounded(5)
          .text_size(11)
          .text_color(o.on ? t.foreground : t.muted)
          .when(o.on, (e) => e.bg(t.selected))
          .tooltip(o.tip)
          .accessibility_label(o.tip)
          .when(o.click !== null, (e) => e.cursor_pointer().when(!o.on, (x) => x.hover((s) => s.bg(t.hover))).on_click((_e, cx) => {
            cx.stop_propagation();
            o.click?.();
          }))
          .when(o.click === null, (e) => e.opacity(0.4))
          .child(o.label),
      ),
    );
}

// A theme colour by the name rows.ts chose; a name the theme lacks is the plain foreground.
const pick = (name: string, t: Theme): `#${string}` => t[name] ?? t.foreground;

// Where a card or the editor stands: the whole stream's width, 48 px in, or one half of a side-by-side stream, 28 px in from the
// half's edge, with the other half left empty and the rule between them drawn as a code row's is. `heightOf` counts the width
// each takes (`cardWidth`, `halfCardWidth`).
function slot(half: Side, t: Theme, box: (e: ReturnType<typeof v_flex>) => Element): Element {
  const one = box(v_flex().h_full().pr(12).pl(half === null ? 48 : 28).w_full().flex_none());
  if (half === null) return one;
  // Each half is a bare flex_1 box, as a code row's halves are: padding on a flex_1 item counts toward its size, so the padded
  // card half would come out wider than the empty one and move the rule off the code rows' rule. The padding goes inside.
  const cell = (child: Element | null): Element => div().flex_1().min_w(0).h_full().overflow_hidden().when(child !== null, (e) => e.child(child as Element));
  return h_flex().w_full().h_full().child(cell(half === "old" ? one : null)).child(div().w(1).h_full().flex_none().bg(t.border)).child(cell(half === "new" ? one : null));
}

function head(item: HeadItem, t: Theme): Element {
  const color = pick(toneColor(item.tone), t);
  return slot(item.half, t, (e) =>
    e.pt(6).child(
      v_flex()
        .flex_1()
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
            .children(item.badges.map((b) => div().flex_none().text_size(10).text_color(pick(tagColor(b), t)).child(`[${b}]`))),
        )
        .when(item.quote !== null, (e) => e.child(div().truncate().text_size(11).text_color(t.dim).child(item.quote ?? ""))),
    ));
}

function body(item: BodyItem, t: Theme, dispatch: Dispatch): Element {
  const color = pick(toneColor(item.tone), t);
  return slot(item.half, t, (e) =>
    e.when(item.last, (x) => x.pb(6)).child(
      v_flex()
        .flex_1()
        .max_w(CARD_W)
        .px(10)
        .py(2)
        .when(item.meta !== null, (e) => e.pl(22))
        .border_l(2)
        .border_color(color)
        .bg(item.focused ? t.selected : tint(color, "14"))
        .when(item.meta !== null, (e) => e.child(div().text_size(10).text_color(t.dim).child(item.meta ?? "")))
        .children(item.lines.map((line) => div().min_h(16).text_size(12).text_color(t.foreground).child(line === "" ? " " : line)))
        // heightOf counts this row: 4 above, 24 a button, 6 between rows when it wraps.
        .when(item.buttons.length > 0, (e) =>
          e.child(h_flex().flex_wrap().gap(6).pt(4).children(item.buttons.map((b) => recordButton(`card-${item.id}-${b.id}`, b, t, dispatch)))),
        ),
    ));
}

// The editor, in the stream under what it comments on: the title, the text box, a line for the status (red, kept free so the
// height is fixed) and the buttons. The keys of the box (Ctrl+S, Esc) are the box's own, as the dock's were. EDITOR_H counts it: 6 px
// above and below, the box with 6 px of padding and 4 px between its four parts.
function editorBox(item: EditorItem, m: Model, t: Theme, dispatch: Dispatch, ui: Ui): Element {
  const ta = ui.textarea;
  // The state is made as the editor opens, so a draw without it is one that came before.
  if (ta === null) return div();
  return slot(item.half, t, (e) =>
    e.pt(6).pb(6).child(
      v_flex()
        .id("review-editor")
        .h(EDITOR_H - 12)
        .flex_none()
        .overflow_hidden()
        .px(10)
        .py(6)
        .gap(4)
        .border_l(2)
        .border_color(t.accent)
        .bg(tint(t.accent, "14"))
        .track_focus(ui.handle)
        .on_key_down((ev, cx) => {
          const msg = editorKey(ev.keystroke, ta.value());
          if (msg !== null) {
            dispatch(msg);
            cx.stop_propagation();
          }
        })
        .child(div().h(16).flex_none().truncate().text_size(11).font_semibold().text_color(pick(toneColor("user"), t)).child(item.title))
        .child(div().h(EDITOR_TEXT).flex_none().child(Textarea.new(ta).w_full().h(EDITOR_TEXT).px(8).py(6).rounded(6).border_1().border_color(t.accent).bg(t.background).text_size(12).text_color(t.foreground)))
        .child(div().h(16).flex_none().truncate().text_size(11).text_color(t.ansi1).child(m.status ?? ""))
        .child(
          h_flex()
            .h(24)
            .flex_none()
            .gap(8)
            .items_center()
            .child(button("editor-save", m.saving ? "Saving..." : "Save", true, t, () => dispatch({ _tag: "EditorSaved", text: ta.value() })))
            .child(button("editor-cancel", "Cancel", false, t, () => dispatch({ _tag: "EditorCancelled" })))
            .child(div().min_w(0).truncate().text_size(11).text_color(t.dim).child("Ctrl+S save, Esc cancel")),
        ),
    ),
  );
}

// The GitHub-style checkbox at the right of a file's header. Its press is its own: it must not also fold the file behind it.
function viewedBox(item: FileItem, t: Theme, dispatch: Dispatch): Element {
  const tip = item.viewed ? "Mark unviewed" : "Mark viewed";
  return h_flex()
    .id(`viewed-${item.key}`)
    .h(22)
    .flex_none()
    .px(6)
    .gap(6)
    .items_center()
    .rounded(6)
    .cursor_pointer()
    .hover((s) => s.bg(t.hover))
    .active((s) => s.bg(t.selected))
    .tooltip(tip)
    .accessibility_label(tip)
    .on_click((_e, cx) => {
      cx.stop_propagation();
      dispatch(viewedClick(item));
    })
    .child(
      div()
        .w(14)
        .h(14)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(3)
        .border_1()
        .border_color(item.viewed ? t.accent : t.dim)
        .when(item.viewed, (e) => e.bg(t.selected).child(svg(drawing("check")).size(10).flex_none().text_color(t.accent))),
    )
    .child(div().text_size(11).text_color(item.viewed ? t.foreground : t.muted).child("Viewed"));
}

function fileHead(item: FileItem, t: Theme, dispatch: Dispatch): Element {
  return h_flex()
    .w_full()
    .h_full()
    .pl(12)
    // The stream's scrollbar track is 12 px wide over its right edge, so the checkbox keeps 12 more clear of it.
    .pr(24)
    .gap(8)
    .items_center()
    .bg(t.raised)
    .border_t(1)
    .border_color(t.border)
    .child(div().w(10).flex_none().text_color(t.dim).child(item.folded ? "\u25b8" : "\u25be"))
    .child(div().w(12).flex_none().text_size(11).text_color(pick(letterColor(item.letter), t)).child(item.letter))
    .child(div().min_w(0).truncate().font_semibold().text_size(12).text_color(item.viewed ? t.dim : t.foreground).child(item.title))
    .when(item.add > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi2).child(`+${item.add}`)))
    .when(item.del > 0, (e) => e.child(div().flex_none().text_size(11).text_color(t.ansi1).child(`-${item.del}`)))
    .when(item.hides !== "", (e) => e.child(div().flex_none().text_size(11).text_color(t.dim).child(item.hides)))
    .child(div().flex_1().min_w(0))
    .child(viewedBox(item, t, dispatch));
}

// One item of the stream, in a box of the height the stream counted for it. A header, a card and a thread take a click of
// their own; a code row takes the hover, the press and the [+], on the row itself.
function item(i: StreamItem, size: number, shift: number, m: Model, t: Theme, dispatch: Dispatch, ui: Ui): Element {
  const click = itemClick(i.key);
  return div()
    .id(`s-${i.key}`)
    .w_full()
    .h(size)
    .flex_none()
    .overflow_hidden()
    .when(click !== null, (e) => e.cursor_pointer().on_click(() => {
      if (click !== null) dispatch(click);
    }))
    .child(inner(i, shift, m, t, dispatch, ui));
}

function inner(i: StreamItem, shift: number, m: Model, t: Theme, dispatch: Dispatch, ui: Ui): Element {
  if (i.kind === "head") return head(i, t);
  if (i.kind === "body") return body(i, t, dispatch);
  if (i.kind === "editor") return editorBox(i, m, t, dispatch, ui);
  if (i.kind === "file") return fileHead(i, t, dispatch);
  if (i.kind === "wait") return div().w_full().pl(48).pt(8).text_size(11).text_color(t.dim).child(i.text);
  if (i.kind === "note") return div().w_full().pl(48).pt(4).text_size(11).text_color(t.dim).child(i.text);
  if (i.kind === "label") return div().w_full().px(12).pt(8).pb(2).text_size(11).font_semibold().text_color(t.dim).child(i.text);
  if (i.kind === "split") return splitRow(i, shift, m, t, dispatch);
  return diffRow(i, null, 0, m, t, dispatch);
}

// A code row is an ordinary element, so it can carry the pointer's events: the hover (and the move, which also reports the
// row when the stream scrolled under a pointer that did not move), the press that starts a drag, and in the gutter the [+],
// drawn only on the hovered line, with a click of its own that does not start a drag.
// `side` is null for a unified row (two number columns, the whole width) and "old" or "new" for one half of a side-by-side
// item (one number column, half the width); the half is the side its events and its tint are for.
function diffRow(item: RowItem, side: "old" | "new" | null, shift: number, m: Model, t: Theme, dispatch: Dispatch): Element {
  const path = keyPath(item.key);
  const over = (on: boolean): void => dispatch({ _tag: "Hovered", path, row: item.index, on, side });
  const key = side === null ? item.key : `${item.key}-${side}`;
  if (item.kind === "hunk") {
    return div().id(`row-${key}`).w_full().px(12).h(ROW).flex_none().truncate().bg(t.raised).text_color(t.dim).on_hover((on) => over(on)).on_mouse_move(() => over(true)).child(item.text);
  }
  const { selected, hovered } = rowState(m, item, side);
  const cursor = cursorOn(m, item, side);
  const sign = item.kind === "add" ? "+" : item.kind === "del" ? "-" : " ";
  const num = (n: string): Element => div().w(36).flex_none().text_right().text_color(t.dim).child(n);
  return h_flex()
    .id(`row-${key}`)
    .h(ROW)
    .flex_none()
    .gap(6)
    .when(side === null, (e) => e.w_full())
    .when(side !== null, (e) => e.flex_1().min_w(0).overflow_hidden())
    .when(item.kind === "add", (e) => e.bg(tint(t.ansi2, "18")))
    .when(item.kind === "del", (e) => e.bg(tint(t.ansi1, "18")))
    .when(selected, (e) => e.bg(tint(t.accent, "30")))
    // The keyboard's row: a neutral tint over the rest, so it reads apart from the accent of a selection.
    .when(cursor, (e) => e.bg(tint(t.foreground, "30")))
    .on_hover((on) => over(on))
    .on_mouse_move(() => over(true))
    .on_mouse_down("left", () => dispatch({ _tag: "RowPressed", path, row: item.index, side }))
    .child(
      div()
        .id(`plus-${key}`)
        .w(GUTTER_W)
        .h_full()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when(hovered, (e) =>
          e
            .cursor_pointer()
            // The press on the [+] is its own: it must not start a drag on the row behind it.
            .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
            .on_click(() => dispatch({ _tag: "PlusClicked", path, row: item.index, side }))
            .child(svg("mods/herdr-review/icons/plus.svg").size(16).flex_none().text_color(pick(PLUS, t))),
        ),
    )
    .when(side !== "new", (e) => e.child(num(item.old)))
    .when(side !== "old", (e) => e.child(num(item.new)))
    .child(div().w(10).flex_none().text_color(item.kind === "add" ? t.ansi2 : item.kind === "del" ? t.ansi1 : t.dim).child(sign))
    .child(
      // A half clips its code and shifts it by the model's shift; the unified stream scrolls sideways as a whole instead.
      div()
        .flex_1()
        .min_w(0)
        .overflow_hidden()
        .child(
          h_flex()
            .when(side !== null, (e) => e.ml(-shift))
            .children(
              // A changed word has its own, stronger background, drawn over the row's tint and the selection's alike.
              item.runs.map((r) =>
                div()
                  .flex_none()
                  .whitespace_nowrap()
                  .text_color(t[r.color] ?? t.foreground)
                  .when(r.marked === true, (e) => e.bg(tint(item.kind === "add" ? t.ansi2 : t.ansi1, "48")))
                  .child(r.text),
              ),
            ),
        ),
    );
}

// A side-by-side item: the old half and the new half with a 1px rule between. A missing half faces the longer run of a
// change and is an empty faint box with no handlers.
function splitRow(i: SplitItem, shift: number, m: Model, t: Theme, dispatch: Dispatch): Element {
  const half = (row: RowItem | null, side: "old" | "new"): Element =>
    row === null ? div().flex_1().min_w(0).h_full().bg(tint(t.dim, "10")) : diffRow(row, side, shift, m, t, dispatch);
  return h_flex()
    .w_full()
    .h(ROW)
    .flex_none()
    .child(half(i.left, "old"))
    .child(div().w(1).h_full().flex_none().bg(t.border))
    .child(half(i.right, "new"));
}

// The bar under the diff: what is selected or focused, the review-wide buttons, a question or the picker. The editor is not here, it is
// in the stream (`editorBox`).
export function dockBar(m: Model, t: Theme, dispatch: Dispatch, ui: Ui): Element {
  const wrap = v_flex().id("review-dock").flex_none().border_t(1).border_color(t.border).px(12).py(8).gap(6).text_size(11);
  if (m.picker !== null) {
    return wrap
      .child(div().font_semibold().text_color(pick(WARN, t)).child("Several agents match this folder. Send to which?"))
      .children(pickerOptions(m.picker).map((b) => h_flex().child(button(`dock-${b.id}`, b.label, false, t, () => dispatch(b.msg)))));
  }
  const btn = (b: DockButton): Element => recordButton(`dock-${b.id}`, b, t, dispatch);
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
      .child(div().font_semibold().text_color(pick(ask.color, t)).child(ask.sentence))
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
    .child(h_flex().gap(8).items_center().flex_wrap().children(bar.actions.map(btn)).when(bar.note !== null, (x) => x.child(div().text_color(pick(bar.note?.color ?? "dim", t)).child(bar.note?.text ?? ""))));
}

function sideRow(r: SideRow, m: Model, t: Theme, dispatch: Dispatch): Element {
  if (r.kind === "dir") return div().w_full().flex_none().px(8).pt(8).pb(2).truncate().text_ellipsis_start().text_size(11).text_color(t.dim).child(r.text);
  const letter = pick(letterColor(r.letter), t);
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
    .when(r.badges.length > 0, (e) => e.child(h_flex().w_full().pl(38).gap(8).children(r.badges.map((b) => div().flex_none().text_size(10).text_color(pick(badgeColor(b), t)).child(b.text)))));
}

// The files by folder in a column of their own, under the filter box. A folder heading is a plain div with no
// handler, so a click on it does nothing. The column has its own scroll handle and bar, laid over a relative box.
function sidebar(m: Model, t: Theme, dispatch: Dispatch, ui: Ui, empty: string): Element {
  const left = m.panel.side === "left";
  const rows = sidebarRows(m.state === null ? NO_FILES : m.state.files, m.filter);
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

// The stream of every file's diff, and under it the bar that holds the selection, the review-wide buttons or the editor.
// The mod scrolls the stream itself: it draws only the items that overlap the screen, as plain children of a clipped box, the
// first one shifted up by the part of it above the top, and its own scrollbar beside them.
function diffPane(m: Model, stream: Stream, t: Theme, dispatch: Dispatch, ui: Ui): Element {
  const { first, end, shift } = visibleRange(stream, m.offset, m.height);
  const kids: Element[] = [];
  // How far a sideways scroll may take the code: until the end of the longest line on screen reaches the edge. It is a limit
  // on the scroll only; the code is drawn at the model's shift, so a vertical scroll never moves it sideways.
  const max = maxShift(codeWidth(stream.items.slice(first, end)), m.width);
  for (let i = first; i < end; i++) {
    const it = stream.items[i];
    if (it !== undefined) kids.push(item(it, stream.sizes[i] ?? ROW, m.shift, m, t, dispatch, ui));
  }
  // A mostly sideways event is left to the box's own sideways scroll, or side by side shifts the halves' code. Any other
  // is the stream's, and must not also move that box sideways, which a vertical wheel over it would.
  const wheel = (e: ScrollWheelEvent, cx: Context): void => {
    const dx = isSplit(m) ? wheelShift(e) : null;
    if (dx !== null) {
      cx.stop_propagation();
      dispatch({ _tag: "Shifted", dx, max });
      return;
    }
    const dy = wheelDelta(e);
    if (dy === null) return;
    cx.stop_propagation();
    dispatch({ _tag: "Wheeled", dy });
  };
  const thumb = thumbOf(stream.total, m.height, m.offset);
  return v_flex()
    .flex_1()
    .min_w(0)
    .h_full()
    .min_h(0)
    .child(
      div()
        .id("review-stream-box")
        .flex_1()
        .min_h(0)
        .relative()
        .overflow_hidden()
        .text_size(11)
        .on_size_change((size) => dispatch({ _tag: "Resized", width: Math.round(size.width), height: Math.round(size.height) }))
        .on_mouse_move((e) => {
          if (m.grip !== null) dispatch({ _tag: "ThumbMoved", y: e.position.y });
        })
        .child(
          div()
            .id("review-stream-x")
            .size_full()
            .overflow_x_scrollbar()
            .child(
              div()
                .id("review-stream-content")
                .w_full()
                .min_w(stream.wide)
                .h_full()
                .relative()
                .overflow_hidden()
                .on_scroll_wheel(wheel)
                .child(v_flex().id("review-stream-items").absolute().top(-shift).left(0).w_full().children(kids)),
            ),
        )
        .when(thumb !== null, (e) =>
          e.child(
            div()
              .id("review-stream-track")
              .absolute()
              .top(0)
              .right(0)
              .w(12)
              .h(m.height)
              .bg(t.chrome)
              .on_scroll_wheel(wheel)
              .on_mouse_down("left", (ev) => {
                if (ev.local_position !== undefined) dispatch({ _tag: "TrackClicked", y: ev.local_position.y });
              })
              .child(
                div()
                  .id("review-stream-thumb")
                  .absolute()
                  .top(thumb?.top ?? 0)
                  .left(2)
                  .w(8)
                  .h(thumb?.size ?? 0)
                  .rounded(4)
                  .bg(m.grip !== null ? t.accent : t.dim)
                  .on_mouse_down("left", (ev, cx) => {
                    cx.stop_propagation();
                    dispatch({ _tag: "ThumbGrabbed", y: ev.position.y });
                  }),
              ),
          ),
        ),
    )
    .child(dockBar(m, t, dispatch, ui));
}

export function draw(m: Model, stream: Stream, t: Theme, dispatch: Dispatch, ui: Ui): Element {
  if (m.view === "message") return drawMessage(m, t, dispatch, ui);
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
    // The button can go up anywhere in the panel, outside the stream too, and that ends a drag and a grip on the thumb.
    .on_mouse_up("left", () => dispatch({ _tag: "Released" }))
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
        // One group that wraps as a unit: the choice of what to diff, the agent's message, a rule, and the four icons.
        .child(
          h_flex()
            .flex_none()
            .gap(8)
            .items_center()
            .child(
              segmented(
                "spec",
                [
                  { id: "spec-worktree", label: "Uncommitted", tip: "Working tree: changes not yet committed (staged, unstaged and untracked)", on: m.spec !== null && m.spec.kind === "worktree", click: () => dispatch({ _tag: "SpecChosen", spec: { kind: "worktree" } }) },
                  {
                    id: "spec-branch",
                    label: base === null ? "Branch" : `vs ${base}`,
                    tip: base === null ? "No base branch found" : `Branch: everything this branch changed since it left ${base}, committed or not`,
                    on: m.spec !== null && m.spec.kind === "branch",
                    click: base === null ? null : () => dispatch({ _tag: "SpecChosen", spec: { kind: "branch", base } }),
                  },
                ],
                t,
              ),
            )
            .child(button("view-message", "Agent message", false, t, () => dispatch({ _tag: "ViewChosen", view: "message" })))
            .child(div().w(1).h(16).flex_none().bg(t.border))
            .children([...sidebarButtons(m), layoutButton(m)].map((b) => iconButton(b.id, b.icon, b.label, b.on ? "on" : "rest", t, () => dispatch(b.msg))))
            .child(iconButton("refresh", "refresh", "Refresh", "rest", t, () => dispatch({ _tag: "Triggered", source: "refresh" }))),
        ),
    )
    .when(m.banner !== null, (e) => e.child(div().px(12).py(6).text_size(12).text_color(t.ansi1).child(m.banner ?? "")))
    .when(m.status !== null, (e) => e.child(div().px(12).py(4).text_size(11).text_color(pick(WARN, t)).child(m.status ?? "")))
    .when(notices.length > 0, (e) => e.child(div().px(12).py(4).text_size(11).text_color(t.dim).child(notices.join(" | "))))
    .child(
      h_flex()
        .flex_1()
        .min_h(0)
        .items_start()
        .children(paneOrder(m).map((p) => (p === "sidebar" ? sidebar(m, t, dispatch, ui, empty) : diffPane(m, stream, t, dispatch, ui)))),
    );
}
