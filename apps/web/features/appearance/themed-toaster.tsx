"use client";

import { useEffect, useState } from "react";
import { Toaster } from "sonner";

/** Toasts in the theme's own surfaces, light or dark. */
export default function ThemedToaster() {
  const [mode, setMode] = useState<"dark" | "light">("dark");
  useEffect(() => {
    const root = document.documentElement;
    const sync = () =>
      setMode(root.getAttribute("data-mode") === "light" ? "light" : "dark");
    sync();
    const observer = new MutationObserver(sync);
    observer.observe(root, {
      attributes: true,
      attributeFilter: ["data-mode"],
    });
    return () => observer.disconnect();
  }, []);
  return (
    <Toaster
      closeButton={false}
      position="bottom-right"
      theme={mode}
      toastOptions={{
        classNames: {
          toast:
            "!border-white/10 !bg-mujin-surface-2 !text-mujin-text !shadow-2xl",
          description: "!text-mujin-text-secondary",
          actionButton: "!bg-mujin-copper !text-mujin-on-copper",
          cancelButton: "!bg-mujin-surface-3 !text-mujin-text",
        },
      }}
    />
  );
}
