import { profileGradient, profileInitials } from "@parson/music-sdk";
import { cn } from "@/lib/cn";

export function ProfileAvatar({
  className,
  color,
  name,
  size = 40,
}: {
  className?: string;
  color: string;
  name: string;
  size?: number;
}) {
  const [from, to] = profileGradient(color);
  return (
    <span
      aria-hidden="true"
      className={cn(
        "relative inline-flex shrink-0 select-none items-center justify-center overflow-hidden rounded-full font-semibold text-white shadow-[inset_0_0_0_1px_rgb(var(--mj-ink)/0.18),0_6px_18px_-8px_rgba(0,0,0,0.9)]",
        className,
      )}
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.38),
        background: `radial-gradient(120% 120% at 25% 15%, ${from} 0%, ${to} 70%)`,
        textShadow: "0 1px 2px rgba(0,0,0,0.35)",
      }}
    >
      {profileInitials(name)}
    </span>
  );
}
