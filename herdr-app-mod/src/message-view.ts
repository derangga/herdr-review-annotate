// GPUI drawing of the agent-message view. Like view.ts it holds no logic: the items come from message.ts, every click is a Msg.
import { div, svg } from "gpui-kit";
import type { Element } from "gpui-kit";
import { h_flex, Scrollbar, v_flex } from "gpui-base";
import { messageItems, sendButton, viewButton } from "./message.ts";
import type { MessageItem } from "./message.ts";
import { CARD_W, GUTTER_W, PLUS, ROW, WARN } from "./rows.ts";
import type { Model, Msg } from "./types.ts";
import { button, dockBar, iconButton } from "./view.ts";
import type { Theme, Ui } from "./view.ts";

type Dispatch = (msg: Msg) => void;
const tint = (color: `#${string}`, alpha: string): `#${string}` => `${color}${alpha}`;
const NUM_W = 36;

function line(i: Extract<MessageItem, { kind: "line" }>, t: Theme, dispatch: Dispatch): Element {
  const over = (on: boolean): void => dispatch({ _tag: "MessageHovered", line: i.n, on });
  return h_flex()
    .id(`msg-${i.n}`)
    .w_full()
    .min_h(ROW)
    .flex_none()
    .items_start()
    .gap(6)
    .when(i.selected, (e) => e.bg(tint(t.accent, "30")))
    .on_hover((on) => over(on))
    .on_mouse_move(() => over(true))
    .on_mouse_down("left", () => dispatch({ _tag: "MessagePressed", line: i.n }))
    .child(
      div()
        .id(`msg-plus-${i.n}`)
        .w(GUTTER_W)
        .h(ROW)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .when(i.hovered, (e) =>
          e
            .cursor_pointer()
            // The press on the [+] is its own: it must not start a drag on the line behind it.
            .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
            .on_click(() => dispatch({ _tag: "MessagePlusClicked", line: i.n }))
            .child(svg("mods/herdr-review/icons/plus.svg").size(16).flex_none().text_color(t[PLUS] ?? t.accent)),
        ),
    )
    .child(div().w(NUM_W).h(ROW).flex_none().text_right().text_size(11).text_color(t.dim).child(String(i.n)))
    // A long line wraps: the message is prose, and clipping it would hide what the user comments on.
    .child(div().flex_1().min_w(0).text_size(12).text_color(t.foreground).child(i.text === "" ? " " : i.text));
}

function card(i: Extract<MessageItem, { kind: "card" }>, t: Theme, dispatch: Dispatch): Element {
  return v_flex()
    .id(`msg-card-${i.id}`)
    .w_full()
    .flex_none()
    .pl(GUTTER_W + NUM_W + 12)
    .pr(12)
    .py(3)
    .child(
      v_flex()
        .flex_1()
        .max_w(CARD_W)
        .px(10)
        .py(4)
        .gap(2)
        .border_l(2)
        .border_color(t.accent)
        .bg(tint(t.accent, "14"))
        .child(
          h_flex()
            .gap(6)
            .items_center()
            .child(div().flex_1().min_w(0).truncate().font_semibold().text_size(11).text_color(t.accent).child(i.label))
            .child(iconButton(`msg-edit-${i.id}`, "pencil", "Edit", "rest", t, () => dispatch({ _tag: "MessageCommentAction", id: i.id, action: "edit" }), 24))
            .child(iconButton(`msg-delete-${i.id}`, "trash", "Delete", "danger", t, () => dispatch({ _tag: "MessageCommentAction", id: i.id, action: "delete" }), 24)),
        )
        .children(i.body.map((text) => div().min_h(16).text_size(12).text_color(t.foreground).child(text === "" ? " " : text))),
    );
}

// The whole panel in the message view: a header with the way back to the diff, the message, and under it the editor or a hint.
export function drawMessage(m: Model, t: Theme, dispatch: Dispatch, ui: Ui): Element {
  const mv = m.message;
  const swap = viewButton(m);
  const send = sendButton(m);
  const items = messageItems(mv);
  const waiting = mv.data === null && mv.status === null;
  return v_flex()
    .id("review")
    .size_full()
    .min_h(0)
    .overflow_hidden()
    .bg(t.background)
    .text_color(t.foreground)
    .text_size(13)
    // The button can go up anywhere in the panel, outside the lines too, and that ends a drag.
    .on_mouse_up("left", () => dispatch({ _tag: "MessageReleased" }))
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
        .child(div().font_semibold().child("Agent message"))
        .when(mv.data !== null, (e) => e.child(div().text_size(11).text_color(t.muted).child(`${mv.data?.agent ?? ""}, ${mv.data?.status ?? ""}`)))
        .child(div().flex_1())
        .child(button(swap.id, swap.label, false, t, () => dispatch(swap.msg)))
        .child(iconButton("message-refresh", "refresh", mv.loading ? "Reading the message..." : "Refresh", "rest", t, mv.loading ? null : () => dispatch({ _tag: "MessageRefreshed" })))
        .child(button("message-send", send.label, !send.off, t, send.off ? null : () => dispatch({ _tag: "MessageSendClicked" }), false, "send")),
    )
    .when(mv.status !== null, (e) => e.child(div().px(12).py(4).text_size(11).text_color(t[WARN] ?? t.foreground).child(mv.status ?? "")))
    .when(mv.loading || waiting, (e) => e.child(div().px(12).py(4).text_size(11).text_color(t.dim).child("Reading the agent's message...")))
    .child(
      div()
        .flex_1()
        .min_h(0)
        .relative()
        .child(
          v_flex()
            .id("review-message")
            .absolute()
            .inset_0()
            .overflow_y_scroll()
            .py(6)
            .children(items.map((i) => (i.kind === "line" ? line(i, t, dispatch) : card(i, t, dispatch)))),
        )
        .child(Scrollbar.vertical("review-message").absolute().inset_0()),
    )
    .child(
      m.editor !== null
        ? dockBar(m, t, dispatch, ui)
        : div().flex_none().border_t(1).border_color(t.border).px(12).py(8).text_size(11).text_color(t.muted).child("Drag over lines to select them, [+] to comment"),
    );
}
