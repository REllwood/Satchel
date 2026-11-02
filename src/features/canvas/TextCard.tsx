import { createContext, memo, useContext, useRef } from "react";
import { Handle, NodeResizer, Position, type Node, type NodeProps } from "@xyflow/react";

import type { CardData } from "./canvas-model";

/** The text card being edited, and how editing ends. */
export interface CardEditing {
  editing: { id: string; fresh: boolean } | null;
  /**
   * Stop editing card `id`, saving `text` as its text (`null` keeps what it
   * had). `fromKeyboard` puts focus back on the card, for keyboard users.
   */
  finish: (id: string, text: string | null, fromKeyboard: boolean) => void;
}

export const CardEditingContext = createContext<CardEditing>({
  editing: null,
  finish: () => {},
});

/** A JSONCanvas text card. Double-click it (or press Enter) to edit its text. */
export const TextCard = memo(function TextCard({
  id,
  data,
  selected,
}: NodeProps<Node<CardData, "text">>) {
  const { editing, finish } = useContext(CardEditingContext);
  const isEditing = editing?.id === id;
  const text = data.canvasNode.text ?? "";
  return (
    <>
      <NodeResizer
        isVisible={selected && !isEditing}
        minWidth={120}
        minHeight={48}
        color="var(--primary)"
      />
      <Handle type="target" position={Position.Top} />
      {isEditing ? (
        <CardTextEditor
          text={text}
          selectAll={editing?.fresh ?? false}
          onDone={(next, fromKeyboard) => finish(id, next, fromKeyboard)}
        />
      ) : (
        <div className="canvas-card-text">
          {text || <span className="text-muted-foreground">Empty card</span>}
        </div>
      )}
      <Handle type="source" position={Position.Bottom} />
    </>
  );
});

function CardTextEditor({
  text,
  selectAll,
  onDone,
}: {
  text: string;
  selectAll: boolean;
  onDone: (text: string | null, fromKeyboard: boolean) => void;
}) {
  // Escape and ⌘↩ end editing before the blur that follows, which must not
  // end it a second time.
  const done = useRef(false);
  const end = (next: string | null, fromKeyboard: boolean) => {
    if (done.current) return;
    done.current = true;
    onDone(next, fromKeyboard);
  };
  return (
    <textarea
      // nodrag/nopan/nowheel: selecting text, scrolling and typing stay in
      // the card instead of moving the canvas.
      className="canvas-card-editor nodrag nopan nowheel"
      aria-label="Card text"
      autoFocus
      defaultValue={text}
      onFocus={(e) => {
        const field = e.currentTarget;
        if (selectAll) field.select();
        else field.setSelectionRange(field.value.length, field.value.length);
      }}
      onBlur={(e) => end(e.currentTarget.value, false)}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault();
          end(null, true);
        } else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          end(e.currentTarget.value, true);
        }
      }}
    />
  );
}
