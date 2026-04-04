import { useEffect, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";

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
  const menuRef = useRef<HTMLDivElement | null>(null);
  const activeOption =
    options.find((option) => option.value === value) ??
    (value ? { value, label: value } : null);

  useEffect(() => {
    if (!open) {
      return;
    }

    function handlePointerDown(event: MouseEvent) {
      if (!menuRef.current?.contains(event.target as Node)) {
        setOpen(false);
      }
    }

    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setOpen(false);
      }
    }

    window.addEventListener("mousedown", handlePointerDown);
    window.addEventListener("keydown", handleEscape);

    return () => {
      window.removeEventListener("mousedown", handlePointerDown);
      window.removeEventListener("keydown", handleEscape);
    };
  }, [open]);

  return (
    <div className={containerClassName || "settings-field"}>
      <label className={labelClassName || "settings-label"} htmlFor={id}>
        {label}
      </label>
      <div className="settings-choice-shell" ref={menuRef}>
        <button
          id={id}
          type="button"
          className={`settings-input settings-choice-button${open ? " is-open" : ""}`}
          aria-haspopup="listbox"
          aria-expanded={open}
          onClick={() => setOpen((current) => !current)}
        >
          <span>{activeOption?.label ?? placeholder ?? ""}</span>
          <ChevronDown size={16} className="settings-choice-icon" />
        </button>
        {open ? (
          <div className="settings-choice-menu" role="listbox">
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
          </div>
        ) : null}
      </div>
    </div>
  );
}
