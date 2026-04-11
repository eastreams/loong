import { useEffect, useId, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { createPortal } from "react-dom";

const CHOICE_FIELD_OPEN_EVENT = "loongclaw:choice-field-open";

export interface ChoiceFieldOption {
  value: string;
  label: string;
}

export function ChoiceField(props: {
  id: string;
  label: string;
  value: string;
  placeholder?: string;
  options: ChoiceFieldOption[];
  onSelect: (value: string) => void;
  containerClassName?: string;
  labelClassName?: string;
}) {
  const {
    id,
    label,
    value,
    placeholder,
    options,
    onSelect,
    containerClassName,
    labelClassName,
  } = props;
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const listboxId = useId();
  const [menuStyle, setMenuStyle] = useState<{ top: number; left: number; width: number } | null>(
    null,
  );
  const activeOption =
    options.find((option) => option.value === value) ??
    (value ? { value, label: value } : null);
  const containerClass = `${containerClassName || "settings-field"} settings-choice-field`;

  useEffect(() => {
    if (!open) {
      return;
    }

    function updateMenuPosition() {
      const rect = triggerRef.current?.getBoundingClientRect();
      if (!rect) {
        return;
      }

      setMenuStyle({
        top: rect.bottom + 7,
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
        setOpen(false);
      }
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setOpen(false);
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
  }, [open]);

  useEffect(() => {
    function handleChoiceFieldOpened(event: Event) {
      const detail = (event as CustomEvent<{ id?: string }>).detail;
      if (detail?.id && detail.id !== id) {
        setOpen(false);
      }
    }

    window.addEventListener(
      CHOICE_FIELD_OPEN_EVENT,
      handleChoiceFieldOpened as EventListener,
    );

    return () => {
      window.removeEventListener(
        CHOICE_FIELD_OPEN_EVENT,
        handleChoiceFieldOpened as EventListener,
      );
    };
  }, [id]);

  useEffect(() => {
    if (!open) {
      return;
    }

    window.dispatchEvent(
      new CustomEvent(CHOICE_FIELD_OPEN_EVENT, {
        detail: { id },
      }),
    );
  }, [id, open]);

  return (
    <div className={containerClass}>
      <label className={labelClassName || "settings-label"} htmlFor={id}>
        {label}
      </label>
      <div className="settings-choice-shell" ref={rootRef}>
        <button
          id={id}
          ref={triggerRef}
          type="button"
          className={`settings-input settings-choice-button${open ? " is-open" : ""}`}
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-controls={listboxId}
          onClick={() => setOpen((current) => !current)}
        >
          <span>{activeOption?.label ?? placeholder ?? ""}</span>
          <ChevronDown size={16} className="settings-choice-icon" />
        </button>
      </div>
      {open && menuStyle && typeof document !== "undefined"
        ? createPortal(
            <div
              ref={menuRef}
              className="settings-choice-menu"
              role="listbox"
              id={listboxId}
              aria-labelledby={id}
              style={{
                top: `${menuStyle.top}px`,
                left: `${menuStyle.left}px`,
                width: `${menuStyle.width}px`,
              }}
            >
              {options.map((option) => (
                <button
                  key={option.value}
                  type="button"
                  role="option"
                  aria-selected={value === option.value}
                  className={`settings-choice-option${
                    value === option.value ? " is-selected" : ""
                  }`}
                  onClick={() => {
                    onSelect(option.value);
                    setOpen(false);
                  }}
                >
                  {option.label}
                </button>
              ))}
            </div>,
            document.body,
          )
        : null}
    </div>
  );
}
