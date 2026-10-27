"use client";

import { validProfilePin } from "@parson/music-sdk";
import { ArrowRight, Loader2 } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { cn } from "@/lib/cn";

/**
 * Unlocks a profile with a PIN (4 to 8 digits, shown as dots) or with the
 * account password.
 */
export function UnlockEntry({
  busy,
  error,
  mode,
  onSubmit,
}: {
  busy: boolean;
  error: string | null;
  mode: "pin" | "password";
  onSubmit: (value: string) => void;
}) {
  const [value, setValue] = useState("");
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
  }, []);
  useEffect(() => {
    if (error) {
      setValue("");
      input.current?.focus();
    }
  }, [error]);
  const ready = mode === "pin" ? validProfilePin(value) : value.length >= 8;
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (ready && !busy) onSubmit(value);
  };
  return (
    <form className="mt-3" onSubmit={submit}>
      <label className="sr-only" htmlFor={`unlock-${mode}`}>
        {mode === "pin" ? "PIN" : "Password"}
      </label>
      <div
        className={cn(
          "relative flex h-11 items-center gap-2 rounded-xl border bg-black/60 pl-3 pr-1.5 transition-colors",
          error
            ? "animate-[pin-shake_0.32s_ease-in-out] border-rose-500/70"
            : "border-white/12 focus-within:border-white/30",
        )}
      >
        {mode === "pin" ? (
          <>
            <input
              autoComplete="off"
              className="absolute inset-0 cursor-text opacity-0"
              id="unlock-pin"
              inputMode="numeric"
              maxLength={8}
              onChange={(event) =>
                setValue(event.target.value.replace(/\D/g, "").slice(0, 8))
              }
              pattern="[0-9]*"
              ref={input}
              type="password"
              value={value}
            />
            <div aria-hidden="true" className="flex flex-1 items-center gap-2">
              {Array.from({ length: Math.max(4, value.length) }, (_, index) => (
                <span
                  className={cn(
                    "h-2.5 w-2.5 rounded-full transition-all duration-150",
                    index < value.length
                      ? "scale-100 bg-white"
                      : "scale-75 bg-white/20",
                  )}
                  key={index}
                />
              ))}
            </div>
          </>
        ) : (
          <input
            autoComplete="current-password"
            className="h-full flex-1 bg-transparent text-sm text-white outline-none placeholder:text-zinc-600"
            id="unlock-password"
            onChange={(event) => setValue(event.target.value)}
            placeholder="Account password"
            ref={input}
            type="password"
            value={value}
          />
        )}
        <button
          aria-label="Switch"
          className="relative z-10 flex h-8 w-8 items-center justify-center rounded-lg bg-white text-black transition-opacity disabled:opacity-25"
          disabled={!ready || busy}
          type="submit"
        >
          {busy ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <ArrowRight className="h-4 w-4" />
          )}
        </button>
      </div>
      <p
        aria-live="polite"
        className={cn(
          "mt-1.5 min-h-4 text-[11px]",
          error ? "text-rose-300" : "text-zinc-500",
        )}
      >
        {error ??
          (mode === "pin"
            ? "Type the profile's PIN, then press Enter."
            : "This account asks for its password.")}
      </p>
    </form>
  );
}
