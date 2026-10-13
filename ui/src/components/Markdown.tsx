import ReactMarkdown from "react-markdown";

/// Model-written markdown. No raw HTML is rendered (react-markdown's default) and no images.
/// A link is shown, with its target as a tooltip, but cannot be followed: a click would
/// navigate the app's own window to a page the model chose.
export function Markdown({ text }: { text: string }) {
  return (
    <div className="flex flex-col gap-2 break-words [&_code]:font-mono [&_code]:text-xs [&_h1]:text-base [&_h1]:font-semibold [&_h2]:font-semibold [&_h3]:font-semibold [&_ol]:list-decimal [&_ol]:pl-5 [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-muted [&_pre]:p-2 [&_ul]:list-disc [&_ul]:pl-5">
      <ReactMarkdown
        disallowedElements={["img"]}
        components={{
          a: ({ children, href }) => (
            <span className="underline decoration-dotted" title={href}>
              {children}
            </span>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  );
}
