import { type KeyboardEvent, useEffect, useLayoutEffect, useRef, useState } from "react";

export interface MenuItem {
  label: string;
  onSelect: () => void;
  danger?: boolean;
}

interface ContextMenuProps {
  /** Names the menu for screen readers, such as the meeting it acts on. */
  label: string;
  items: MenuItem[];
  /** Where it opens, in window coordinates. */
  x: number;
  y: number;
  /** Called when it closes, after an item or by dismissing it. `restoreFocus`
      is true when focus should go back to what opened it. */
  onClose: (restoreFocus: boolean) => void;
}

/** Keeps the menu this far from the window's edges. */
const EDGE = 8;

/**
 * A menu that opens in place of the webview's own. It takes focus, moves with
 * the arrow keys, and closes on Escape, Tab, a click elsewhere, or a scroll.
 */
export function ContextMenu({ label, items, x, y, onClose }: ContextMenuProps) {
  const menu = useRef<HTMLUListElement>(null);
  const [position, setPosition] = useState({ left: x, top: y });

  // Keep the whole menu in the window, then focus its first item.
  useLayoutEffect(() => {
    const element = menu.current;
    if (!element) return;
    const { width, height } = element.getBoundingClientRect();
    setPosition({
      left: Math.max(EDGE, Math.min(x, window.innerWidth - width - EDGE)),
      top: Math.max(EDGE, Math.min(y, window.innerHeight - height - EDGE)),
    });
    element.querySelector<HTMLButtonElement>("[role=menuitem]")?.focus();
  }, [x, y]);

  useEffect(() => {
    const dismiss = (event: Event) => {
      if (event.target instanceof Node && menu.current?.contains(event.target)) return;
      onClose(false);
    };
    const close = () => onClose(false);
    document.addEventListener("pointerdown", dismiss, true);
    document.addEventListener("scroll", close, true);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      document.removeEventListener("pointerdown", dismiss, true);
      document.removeEventListener("scroll", close, true);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
    };
  }, [onClose]);

  function move(event: KeyboardEvent<HTMLUListElement>) {
    const buttons = [...(menu.current?.querySelectorAll<HTMLButtonElement>("[role=menuitem]") ?? [])];
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const last = buttons.length - 1;
    const next: Record<string, number> = {
      ArrowDown: current >= last ? 0 : current + 1,
      ArrowUp: current <= 0 ? last : current - 1,
      Home: 0,
      End: last,
    };
    if (event.key in next) {
      event.preventDefault();
      buttons[next[event.key]]?.focus();
    } else if (event.key === "Escape" || event.key === "Tab") {
      event.preventDefault();
      onClose(true);
    }
  }

  return (
    <ul
      ref={menu}
      role="menu"
      aria-label={label}
      className="context-menu"
      style={position}
      onKeyDown={move}
      onContextMenu={(event) => event.preventDefault()}
    >
      {items.map((item) => (
        <li key={item.label} role="none">
          <button
            type="button"
            role="menuitem"
            tabIndex={-1}
            className={`context-menu-item${item.danger ? " is-danger" : ""}`}
            onClick={() => {
              // Back to the meeting first; a dialog the item opens takes it from there.
              onClose(true);
              item.onSelect();
            }}
          >
            {item.label}
          </button>
        </li>
      ))}
    </ul>
  );
}
