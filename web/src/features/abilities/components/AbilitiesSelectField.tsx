import { ChevronDown } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";

export interface AbilitiesSelectOption {
  value: string;
  label: string;
}

export function AbilitiesSelectField(props: {
  id: string;
  label: string;
  value: string;
  options: AbilitiesSelectOption[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onChange: (value: string) => void;
}) {
  const { id, label, value, options, open, onOpenChange, onChange } = props;
  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const listboxId = useId();
  const [menuStyle, setMenuStyle] = useState<{ top: number; left: number; width: number } | null>(
    null,
  );
  const selectedOption = options.find((option) => option.value === value) ?? options[0];

  useEffect(() => {
    if (!open) {
      return undefined;
    }

    function updateMenuPosition() {
      const rect = triggerRef.current?.getBoundingClientRect();
      if (!rect) {
        return;
      }
      setMenuStyle({
        top: rect.bottom - 1,
        left: rect.left,
        width: rect.width,
      });
    }

    updateMenuPosition();

    function handlePointerDown(event: MouseEvent) {
      const target = event.target as Node;
      const withinTrigger = rootRef.current?.contains(target);
      const withinMenu = menuRef.current?.contains(target);
      if (!withinTrigger && !withinMenu) {
        onOpenChange(false);
      }
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onOpenChange(false);
      }
    }

    document.addEventListener("mousedown", handlePointerDown);
    document.addEventListener("keydown", handleEscape);
    window.addEventListener("resize", updateMenuPosition);
    window.addEventListener("scroll", updateMenuPosition, true);
    return () => {
      document.removeEventListener("mousedown", handlePointerDown);
      document.removeEventListener("keydown", handleEscape);
      window.removeEventListener("resize", updateMenuPosition);
      window.removeEventListener("scroll", updateMenuPosition, true);
    };
  }, [onOpenChange, open]);

  function selectValue(nextValue: string) {
    onChange(nextValue);
    onOpenChange(false);
  }

  return (
    <div className="abilities-select-row">
      <label className="abilities-form-label" htmlFor={id}>
        {label}
      </label>
      <div
        ref={rootRef}
        className={`abilities-select-shell${open ? " is-open" : ""}`}
      >
        <button
          id={id}
          ref={triggerRef}
          type="button"
          className="abilities-select-trigger"
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-controls={listboxId}
          onClick={() => onOpenChange(!open)}
        >
          <span className="abilities-select-value">{selectedOption?.label ?? ""}</span>
          <ChevronDown size={16} className="abilities-select-icon" />
        </button>
        {open && menuStyle
          ? createPortal(
              <div
                ref={menuRef}
                className="abilities-select-menu"
                role="listbox"
                id={listboxId}
                aria-labelledby={id}
                style={{
                  top: `${menuStyle.top}px`,
                  left: `${menuStyle.left}px`,
                  width: `${menuStyle.width}px`,
                }}
              >
                {options.map((option) => {
                  const isSelected = option.value === value;
                  return (
                <button
                  key={option.value}
                  type="button"
                  role="option"
                  aria-selected={isSelected}
                      className={`abilities-select-option${isSelected ? " is-selected" : ""}`}
                      onClick={() => selectValue(option.value)}
                >
                  <span>{option.label}</span>
                </button>
              );
            })}
              </div>,
              document.body,
            )
          : null}
      </div>
    </div>
  );
}
