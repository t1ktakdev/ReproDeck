import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { selectKeyboardTransition } from "../lib/uiBehavior";

export type SelectOption<T extends string | number> = { value: T; label: string; description?: string };

export function Select<T extends string | number>({ value, options, onChange, ariaLabel, disabled = false }: {
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  ariaLabel: string;
  disabled?: boolean;
}) {
  const id = useId();
  const rootRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const selectedIndex = Math.max(0, options.findIndex(option => option.value === value));
  const [activeIndex, setActiveIndex] = useState(selectedIndex);

  useEffect(() => setActiveIndex(selectedIndex), [selectedIndex]);
  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [open]);

  function choose(index: number) {
    const option = options[index];
    if (!option) return;
    onChange(option.value);
    setOpen(false);
  }

  function onKeyDown(event: React.KeyboardEvent<HTMLButtonElement>) {
    const next = selectKeyboardTransition({ open, activeIndex }, event.key, options.length);
    if (!next) return;
    event.preventDefault();
    setOpen(next.open);
    setActiveIndex(next.activeIndex);
    if (next.chooseIndex !== null) choose(next.chooseIndex);
  }

  return <div className={`ui-select ${open ? "open" : ""}`} ref={rootRef}>
    <button
      type="button"
      className="ui-select-trigger"
      aria-label={ariaLabel}
      aria-haspopup="listbox"
      aria-expanded={open}
      aria-controls={`${id}-list`}
      aria-activedescendant={open ? `${id}-${activeIndex}` : undefined}
      disabled={disabled}
      onClick={() => setOpen(current => !current)}
      onKeyDown={onKeyDown}
    >
      <span>{options[selectedIndex]?.label ?? String(value)}</span><i aria-hidden="true"/>
    </button>
    <div className={`ui-select-menu ${open ? "menu-open" : "menu-closed"}`} id={`${id}-list`} role="listbox" aria-label={ariaLabel} aria-hidden={!open}>
      {options.map((option, index) => <button
        type="button"
        id={`${id}-${index}`}
        role="option"
        aria-selected={option.value === value}
        tabIndex={-1}
        className={index === activeIndex ? "active" : ""}
        key={option.value}
        onMouseEnter={() => setActiveIndex(index)}
        onClick={() => choose(index)}
      ><span><strong>{option.label}</strong>{option.description && <small>{option.description}</small>}</span><b aria-hidden="true">{option.value === value ? "✓" : ""}</b></button>)}
    </div>
  </div>;
}

export function Toggle({ checked, onChange, label, disabled = false }: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return <button type="button" className="ui-toggle" role="switch" aria-checked={checked} aria-label={label} disabled={disabled} onClick={() => onChange(!checked)}><span/></button>;
}

export function SettingRow({ label, description, children, disabled = false }: {
  label: string;
  description: string;
  children: ReactNode;
  disabled?: boolean;
}) {
  return <div className={`setting-row ${disabled ? "disabled" : ""}`}><div><strong>{label}</strong><p>{description}</p></div><div className="setting-control">{children}</div></div>;
}

export function SegmentedControl<T extends string | number>({ value, options, onChange, ariaLabel }: {
  value: T;
  options: SelectOption<T>[];
  onChange: (value: T) => void;
  ariaLabel: string;
}) {
  return <div className="ui-segmented" role="group" aria-label={ariaLabel}>{options.map(option => <button type="button" key={option.value} className={value === option.value ? "active" : ""} aria-pressed={value === option.value} onClick={() => onChange(option.value)}>{option.label}</button>)}</div>;
}

export function Tooltip({ label, shortcut, children }: { label: string; shortcut?: string; children: ReactNode }) {
  return <span className="ui-tooltip"><span className="ui-tooltip-anchor">{children}</span><span className="ui-tooltip-content" role="tooltip">{label}{shortcut && <kbd>{shortcut}</kbd>}</span></span>;
}

export function Spinner({ label }: { label: string }) {
  return <span className="ui-spinner" role="status" aria-label={label}/>;
}

export function ResizeHandle({ side, value, min, max, label, onChange, onCommit }: {
  side: "left" | "right";
  value: number;
  min: number;
  max: number;
  label: string;
  onChange: (value: number) => void;
  onCommit: (value: number) => void;
}) {
  function begin(event: React.PointerEvent<HTMLDivElement>) {
    event.preventDefault();
    const origin = event.clientX;
    const start = value;
    let latest = value;
    const move = (next: PointerEvent) => {
      const delta = side === "left" ? origin - next.clientX : next.clientX - origin;
      latest = Math.max(min, Math.min(max, Math.round(start + delta)));
      onChange(latest);
    };
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      document.body.classList.remove("is-resizing");
      onCommit(latest);
    };
    document.body.classList.add("is-resizing");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end, { once: true });
  }
  return <div className={`resize-handle resize-${side}`} role="separator" aria-orientation="vertical" aria-label={label} aria-valuemin={min} aria-valuemax={max} aria-valuenow={value} tabIndex={0} onPointerDown={begin} onKeyDown={event => {
    const direction = event.key === "ArrowLeft" ? -1 : event.key === "ArrowRight" ? 1 : 0;
    if (!direction) return;
    event.preventDefault();
    const next = Math.max(min, Math.min(max, value + direction * 12));
    onChange(next);
    onCommit(next);
  }}/>;
}
