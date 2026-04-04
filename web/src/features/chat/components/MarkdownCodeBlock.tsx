import { Prism as SyntaxHighlighter } from "react-syntax-highlighter";
import { vscDarkPlus, oneLight } from "react-syntax-highlighter/dist/esm/styles/prism";
import { CopyButton } from "../../../components/feedback/CopyButton";
import { THEMES } from "../../../contexts/ThemeContextValue";
import { useTheme } from "../../../hooks/useTheme";

interface MarkdownCodeBlockProps {
  codeString: string;
  language: string;
}

export default function MarkdownCodeBlock({
  codeString,
  language,
}: MarkdownCodeBlockProps) {
  const { theme } = useTheme();
  const baseTheme = theme === THEMES.DARK ? vscDarkPlus : oneLight;
  const syntaxTheme = {
    ...baseTheme,
    'pre[class*="language-"]': {
      ...baseTheme['pre[class*="language-"]'],
      background: "transparent",
      backgroundColor: "transparent",
    },
    'code[class*="language-"]': {
      ...baseTheme['code[class*="language-"]'],
      background: "transparent",
      backgroundColor: "transparent",
    },
  };

  return (
    <div style={{ position: "relative" }}>
      <div style={{ position: "absolute", top: "0.5rem", right: "0.5rem", zIndex: 10 }}>
        <CopyButton text={codeString} className="message-action-btn" />
      </div>
      <SyntaxHighlighter
        style={syntaxTheme as any}
        language={language}
        PreTag="div"
        customStyle={{
          margin: "0.75rem 0",
          borderRadius: "0.5rem",
          backgroundColor: "var(--color-bg-surface-active)",
          padding: "1rem",
          fontSize: "0.95em",
        }}
        codeTagProps={{
          style: { backgroundColor: "transparent" },
        }}
      >
        {codeString}
      </SyntaxHighlighter>
    </div>
  );
}
