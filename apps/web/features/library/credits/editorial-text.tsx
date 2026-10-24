"use client";

import type { EditorialText as EditorialTextValue } from "@parson/music-sdk/types";
import { ChevronDown } from "lucide-react";
import { useState } from "react";
import { paragraphs } from "./credit-roles";

const SOURCE_NAMES: Record<string, string> = { wikipedia: "Wikipedia" };

/** Attribution required by CC BY-SA: title, link, license and its link. */
export function EditorialAttribution({ text }: { text: EditorialTextValue }) {
  const source = SOURCE_NAMES[text.source] ?? text.source;
  return (
    <p className="text-xs leading-5 text-zinc-500">
      From the {source} article{" "}
      <a
        className="text-zinc-400 underline-offset-2 hover:text-white hover:underline"
        href={text.url}
        rel="noreferrer"
        target="_blank"
      >
        “{text.title}”
      </a>
      {text.language !== "en" && ` (${text.language})`}, licensed under{" "}
      <a
        className="text-zinc-400 underline-offset-2 hover:text-white hover:underline"
        href={text.license_url}
        rel="noreferrer license"
        target="_blank"
      >
        {text.license}
      </a>
      .
    </p>
  );
}

/** Long-form editorial text that starts clamped, like Roon's review block. */
export function EditorialText({
  text,
  clampLines = 4,
}: {
  text: EditorialTextValue;
  clampLines?: number;
}) {
  const [expanded, setExpanded] = useState(false);
  const blocks = paragraphs(text.body);
  const long = text.body.length > 360 || blocks.length > 1;
  return (
    <div className="space-y-3">
      <div
        className={`space-y-3 text-[15px] leading-7 text-zinc-300 ${
          expanded || !long ? "" : "overflow-hidden"
        }`}
        style={
          expanded || !long
            ? undefined
            : {
                display: "-webkit-box",
                WebkitBoxOrient: "vertical",
                WebkitLineClamp: clampLines,
              }
        }
      >
        {(expanded ? blocks : blocks.slice(0, long ? 2 : blocks.length)).map(
          (paragraph, index) => (
            <p key={index}>{paragraph}</p>
          ),
        )}
      </div>
      {long && (
        <button
          aria-expanded={expanded}
          className="flex items-center gap-1 text-sm font-medium text-zinc-400 transition-colors hover:text-white"
          onClick={() => setExpanded((value) => !value)}
          type="button"
        >
          {expanded ? "Show less" : "Read more"}
          <ChevronDown
            className={`h-4 w-4 transition-transform ${expanded ? "rotate-180" : ""}`}
          />
        </button>
      )}
      <EditorialAttribution text={text} />
    </div>
  );
}
