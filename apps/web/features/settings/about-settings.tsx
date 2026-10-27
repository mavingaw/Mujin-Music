import ParsonBrandMark from "@/components/icons/parson-brand-mark";

export const ATTRIBUTION = "Based on Parson by ParsonLabs";

const links = [
  ["Source code", "https://github.com/mavingaw/Mujin-Music"],
  ["Parson (upstream project)", "https://github.com/ParsonLabs/Parson"],
  ["GNU GPL version 3", "https://www.gnu.org/licenses/gpl-3.0.html"],
] as const;

export default function AboutSettings() {
  return (
    <div>
      <div className="flex items-center gap-4">
        <ParsonBrandMark className="h-12 w-12 shrink-0" />
        <div>
          <h2 className="text-xl font-medium text-white">Mujin Music</h2>
          <p className="text-sm text-zinc-400">{ATTRIBUTION}.</p>
        </div>
      </div>
      <p className="mt-5 max-w-xl text-sm text-zinc-400">
        Mujin Music is free software under the GNU General Public License
        version 3 only, like the Parson project it is based on. Its copyright
        notices and the third-party notices travel with every package. The Jost,
        Inter and JetBrains Mono fonts are used under the SIL Open Font License
        1.1 and are served by this app, never fetched from elsewhere.
      </p>
      <ul className="mt-5 grid gap-2 text-sm">
        {links.map(([label, href]) => (
          <li key={href}>
            <a
              className="text-zinc-200 underline-offset-4 hover:text-white hover:underline"
              href={href}
              rel="noreferrer"
              target="_blank"
            >
              {label}
            </a>
          </li>
        ))}
      </ul>
    </div>
  );
}
