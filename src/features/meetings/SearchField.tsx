interface SearchFieldProps {
  value: string;
  onChange: (value: string) => void;
}

/** The sidebar's search box. Escape clears it. */
export function SearchField({ value, onChange }: SearchFieldProps) {
  return (
    <input
      type="search"
      className="search-input"
      aria-label="Search meetings"
      placeholder="Search"
      spellCheck={false}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === "Escape" && value !== "") {
          event.preventDefault();
          onChange("");
        }
      }}
    />
  );
}
