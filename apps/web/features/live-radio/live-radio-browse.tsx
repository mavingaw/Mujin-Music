"use client";

import {
  liveCountryName,
  type LiveCategory,
  type LiveOrder,
  type LiveSearch,
  type LiveStation,
} from "@parson/music-sdk";
import {
  Loader2,
  Plus,
  Radio,
  Search,
  SlidersHorizontal,
  X,
} from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { usePageTitle } from "@/components/app/title-metadata";
import { InfiniteLoad } from "@/features/library/library-view-primitives";
import { cn } from "@/lib/cn";
import { AddStationDialog } from "./add-station-dialog";
import {
  categorySearch,
  hasFilters,
  paramsFromSearch,
  searchFromParams,
  searchTitle,
} from "./live-radio-model";
import { CategoryTile, StationGrid, StationShelf } from "./station-ui";
import {
  liveRadioError,
  useLiveFacets,
  useLiveRadioHome,
  useLiveSearch,
} from "./use-live-radio";

export function LiveRadioTabs({ active }: { active: "browse" | "my" }) {
  const tab = (href: string, label: string, selected: boolean) => (
    <Link
      aria-current={selected ? "page" : undefined}
      className={cn(
        "inline-flex h-9 items-center rounded-full px-4 text-sm font-semibold transition-colors",
        selected
          ? "bg-white text-black"
          : "text-zinc-400 hover:bg-white/8 hover:text-white",
      )}
      href={href}
    >
      {label}
    </Link>
  );
  return (
    <nav aria-label="Live radio" className="flex gap-1">
      {tab("/radio", "Live Radio", active === "browse")}
      {tab("/radio/my", "My Live Radio", active === "my")}
    </nav>
  );
}

export function LiveRadioHeader({
  active,
  title,
  eyebrow,
  children,
}: {
  active: "browse" | "my";
  title: string;
  eyebrow?: string;
  children?: ReactNode;
}) {
  const [adding, setAdding] = useState(false);
  return (
    <header className="relative mb-8">
      <div
        aria-hidden="true"
        className="pointer-events-none absolute -inset-x-40 -top-24 -z-10 h-72 bg-[radial-gradient(60%_80%_at_20%_0%,rgba(244,63,94,0.18),transparent_70%),radial-gradient(50%_70%_at_80%_10%,rgba(124,58,237,0.16),transparent_70%)]"
      />
      <div className="flex flex-wrap items-center justify-between gap-3">
        <LiveRadioTabs active={active} />
        <button
          className="inline-flex h-9 items-center gap-2 rounded-full border border-white/15 px-4 text-sm font-semibold text-white transition-colors hover:border-white/35 hover:bg-white/5"
          onClick={() => setAdding(true)}
          type="button"
        >
          <Plus className="h-4 w-4" /> Add station
        </button>
      </div>
      <p className="mt-8 flex items-center gap-2 text-xs font-semibold uppercase tracking-[0.16em] text-rose-300/80">
        <Radio className="h-3.5 w-3.5" /> {eyebrow ?? "On air worldwide"}
      </p>
      <h1 className="mt-2 text-4xl font-black tracking-tight text-white sm:text-6xl">
        {title}
      </h1>
      {children}
      <AddStationDialog onOpenChange={setAdding} open={adding} />
    </header>
  );
}

function FilterSelect({
  label,
  value,
  onChange,
  children,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  children: ReactNode;
}) {
  return (
    <label
      className={cn(
        "relative inline-flex h-9 items-center rounded-full border text-sm transition-colors",
        value
          ? "border-white/40 bg-white/10 text-white"
          : "border-white/12 text-zinc-300 hover:border-white/25",
      )}
    >
      <span className="sr-only">{label}</span>
      <select
        className="h-full max-w-48 appearance-none truncate bg-transparent pl-3.5 pr-8 outline-none [&>option]:bg-zinc-900"
        onChange={(event) => onChange(event.target.value)}
        value={value}
      >
        {children}
      </select>
      <SlidersHorizontal className="pointer-events-none absolute right-3 h-3.5 w-3.5 opacity-60" />
    </label>
  );
}

const QUALITY_CHOICES = [
  { value: "", label: "Any quality" },
  { value: "lossless", label: "Lossless (FLAC)" },
  { value: "256", label: "256 kbps and up" },
  { value: "128", label: "128 kbps and up" },
];

const ORDER_CHOICES: { value: LiveOrder; label: string }[] = [
  { value: "popular", label: "Most listened" },
  { value: "trending", label: "Trending" },
  { value: "votes", label: "Most loved" },
  { value: "bitrate", label: "Highest bitrate" },
  { value: "name", label: "Name" },
  { value: "random", label: "Surprise me" },
];

function SearchBar({
  search,
  onChange,
}: {
  search: LiveSearch;
  onChange: (search: LiveSearch) => void;
}) {
  const [text, setText] = useState(search.name ?? "");
  useEffect(() => setText(search.name ?? ""), [search.name]);
  useEffect(() => {
    const value = text.trim();
    if (value === (search.name ?? "")) return;
    const timer = setTimeout(
      () => onChange({ ...search, name: value || undefined }),
      350,
    );
    return () => clearTimeout(timer);
  }, [onChange, search, text]);
  const countries = useLiveFacets("countries");
  const languages = useLiveFacets("languages");
  const quality = search.lossless
    ? "lossless"
    : search.min_bitrate
      ? String(search.min_bitrate)
      : "";
  const filtered = hasFilters(search);
  return (
    <div className="mt-6 space-y-3">
      <label className="relative flex h-13 items-center rounded-2xl border border-white/10 bg-white/[0.04] shadow-[0_10px_40px_-20px_rgba(0,0,0,0.8)] transition-colors focus-within:border-white/30 focus-within:bg-white/[0.07]">
        <Search className="ml-4 h-5 w-5 text-zinc-500" />
        <span className="sr-only">Search stations</span>
        <input
          className="h-full min-w-0 flex-1 bg-transparent px-3 text-base text-white outline-none placeholder:text-zinc-500"
          onChange={(event) => setText(event.target.value)}
          placeholder="Search 50,000 stations by name"
          type="search"
          value={text}
        />
        {text && (
          <button
            aria-label="Clear search"
            className="mr-3 grid h-8 w-8 place-items-center rounded-full text-zinc-400 hover:bg-white/10 hover:text-white"
            onClick={() => setText("")}
            type="button"
          >
            <X className="h-4 w-4" />
          </button>
        )}
      </label>
      <div className="-mx-5 flex items-center gap-2 overflow-x-auto px-5 pb-1 sm:mx-0 sm:flex-wrap sm:overflow-visible sm:px-0 [&>*]:shrink-0">
        <FilterSelect
          label="Country"
          onChange={(value) =>
            onChange({ ...search, country_code: value || undefined })
          }
          value={search.country_code ?? ""}
        >
          <option value="">All countries</option>
          {(countries.data ?? [])
            .filter((facet) => facet.code)
            .map((facet) => (
              <option key={facet.code} value={facet.code!.toUpperCase()}>
                {liveCountryName(facet.code, facet.name)} ·{" "}
                {facet.station_count.toLocaleString()}
              </option>
            ))}
        </FilterSelect>
        <FilterSelect
          label="Language"
          onChange={(value) =>
            onChange({ ...search, language: value || undefined })
          }
          value={search.language ?? ""}
        >
          <option value="">All languages</option>
          {(languages.data ?? []).slice(0, 120).map((facet) => (
            <option key={facet.name} value={facet.name}>
              {facet.name.replace(/^\p{L}/u, (letter) => letter.toUpperCase())}{" "}
              · {facet.station_count.toLocaleString()}
            </option>
          ))}
        </FilterSelect>
        <FilterSelect
          label="Quality"
          onChange={(value) =>
            onChange({
              ...search,
              lossless: value === "lossless" || undefined,
              min_bitrate:
                value && value !== "lossless" ? Number(value) : undefined,
            })
          }
          value={quality}
        >
          {QUALITY_CHOICES.map((choice) => (
            <option key={choice.value} value={choice.value}>
              {choice.label}
            </option>
          ))}
        </FilterSelect>
        <FilterSelect
          label="Format"
          onChange={(value) =>
            onChange({ ...search, codec: value || undefined })
          }
          value={search.codec ?? ""}
        >
          <option value="">Any format</option>
          <option value="flac">FLAC</option>
          <option value="aac">AAC</option>
          <option value="mp3">MP3</option>
          <option value="ogg">Ogg Vorbis</option>
          <option value="opus">Opus</option>
        </FilterSelect>
        {filtered && (
          <FilterSelect
            label="Sort"
            onChange={(value) =>
              onChange({ ...search, order: (value || undefined) as LiveOrder })
            }
            value={search.order ?? "popular"}
          >
            {ORDER_CHOICES.map((choice) => (
              <option key={choice.value} value={choice.value}>
                {choice.label}
              </option>
            ))}
          </FilterSelect>
        )}
        {filtered && (
          <button
            className="h-9 rounded-full px-3 text-sm text-zinc-400 hover:text-white"
            onClick={() => onChange({})}
            type="button"
          >
            Clear
          </button>
        )}
      </div>
    </div>
  );
}

function Section({
  title,
  action,
  children,
}: {
  title: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mb-12">
      <div className="mb-4 flex items-end justify-between gap-3 border-b border-white/8 pb-3">
        <h2 className="text-xl font-bold tracking-tight text-white sm:text-2xl">
          {title}
        </h2>
        {action}
      </div>
      {children}
    </section>
  );
}

function MoreLink({
  search,
  label = "More",
}: {
  search: LiveSearch;
  label?: string;
}) {
  return (
    <Link
      className="rounded-full bg-white/8 px-3 py-1 text-xs font-semibold uppercase tracking-wide text-zinc-300 transition-colors hover:bg-white/15 hover:text-white"
      href={`/radio?${paramsFromSearch(search)}`}
    >
      {label}
    </Link>
  );
}

function Categories({ categories }: { categories: LiveCategory[] }) {
  const group = (name: LiveCategory["group"]) =>
    categories.filter((category) => category.group === name);
  const href = (category: LiveCategory) =>
    `/radio?${paramsFromSearch(categorySearch(category))}`;
  return (
    <>
      <Section title="Collections">
        <div className="grid gap-3 sm:grid-cols-3">
          {group("collections").map((category) => (
            <CategoryTile
              category={category}
              href={href(category)}
              key={category.id}
              large
            />
          ))}
        </div>
      </Section>
      <Section title="Music">
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-6">
          {group("music").map((category) => (
            <CategoryTile
              category={category}
              href={href(category)}
              key={category.id}
            />
          ))}
        </div>
      </Section>
      <Section title="News, talk & sports">
        <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
          {group("talk").map((category) => (
            <CategoryTile
              category={category}
              href={href(category)}
              key={category.id}
            />
          ))}
        </div>
      </Section>
    </>
  );
}

function Landing() {
  const home = useLiveRadioHome();
  if (home.isPending) {
    return (
      <div className="flex justify-center pt-16 text-zinc-500">
        <Loader2 className="h-6 w-6 animate-spin" />
      </div>
    );
  }
  if (home.isError || !home.data) {
    return (
      <p className="rounded-xl border border-white/10 p-6 text-sm text-zinc-400">
        {liveRadioError(home.error, "Live radio is unavailable right now.")}
      </p>
    );
  }
  const data = home.data;
  const shelf = (
    title: string,
    stations: LiveStation[],
    more?: LiveSearch,
    href?: string,
  ) =>
    stations.length > 0 && (
      <Section
        action={
          href ? (
            <Link
              className="rounded-full bg-white/8 px-3 py-1 text-xs font-semibold uppercase tracking-wide text-zinc-300 hover:bg-white/15 hover:text-white"
              href={href}
            >
              See all
            </Link>
          ) : more ? (
            <MoreLink search={more} />
          ) : undefined
        }
        title={title}
      >
        <StationShelf label={title} stations={stations} />
      </Section>
    );
  const local = data.local[0]?.country_code;
  return (
    <>
      {data.directory_error && (
        <p className="mb-8 rounded-xl border border-amber-400/20 bg-amber-500/5 px-4 py-3 text-sm text-amber-200/90">
          {data.directory_error} Your saved stations still play.
        </p>
      )}
      {shelf("Recently played", data.recent)}
      {shelf("My Live Radio", data.my, undefined, "/radio/my")}
      {shelf("Popular now", data.popular, { order: "popular" })}
      {shelf("Lossless stations", data.lossless, {
        lossless: true,
        order: "votes",
      })}
      {data.categories.length > 0 && (
        <Categories categories={data.categories} />
      )}
      {shelf(
        local ? `Near you · ${liveCountryName(local) ?? local}` : "Near you",
        data.local,
        local ? { country_code: local } : undefined,
      )}
      {shelf("Trending", data.trending, { order: "trending" })}
      <p className="mt-4 text-xs leading-5 text-zinc-600">
        Station directory by the community at{" "}
        <a
          className="underline-offset-2 hover:text-zinc-400 hover:underline"
          href="https://www.radio-browser.info"
          rel="noreferrer"
          target="_blank"
        >
          radio-browser.info
        </a>
        . Streams play through your Parson server; stations never see your
        devices.
      </p>
    </>
  );
}

function Results({ search }: { search: LiveSearch }) {
  const query = useLiveSearch(search);
  const home = useLiveRadioHome();
  const stations = useMemo(() => {
    const seen = new Set<string>();
    return (query.data?.pages ?? [])
      .flatMap((page) => page.items)
      .filter((station) => !seen.has(station.id) && seen.add(station.id));
  }, [query.data]);
  const title = searchTitle(search, home.data?.categories ?? [], (code) =>
    liveCountryName(code),
  );
  return (
    <Section
      title={title}
      action={
        query.data && (
          <span className="text-sm tabular-nums text-zinc-500">
            {stations.length}
            {query.hasNextPage ? "+" : ""} stations
          </span>
        )
      }
    >
      {query.isPending && (
        <div className="flex justify-center pt-12 text-zinc-500">
          <Loader2 className="h-6 w-6 animate-spin" />
        </div>
      )}
      {query.isError && (
        <p className="text-sm text-zinc-400">
          {liveRadioError(query.error, "The directory could not be searched.")}
        </p>
      )}
      {query.data && stations.length === 0 && (
        <div className="rounded-xl border border-dashed border-white/10 px-6 py-14 text-center">
          <p className="text-lg font-semibold text-white">No stations found</p>
          <p className="mx-auto mt-2 max-w-sm text-sm text-zinc-400">
            Try fewer filters, or add the station yourself with its stream URL.
          </p>
        </div>
      )}
      {stations.length > 0 && <StationGrid label={title} stations={stations} />}
      <InfiniteLoad
        hasMore={Boolean(query.hasNextPage)}
        loading={query.isFetchingNextPage}
        onLoadMore={() => void query.fetchNextPage()}
      />
    </Section>
  );
}

/** Live Radio: the community directory, categories and search. */
export default function LiveRadioBrowse() {
  const params = useSearchParams();
  const router = useRouter();
  const pathname = usePathname();
  const search = useMemo(
    () => searchFromParams(new URLSearchParams(params?.toString() ?? "")),
    [params],
  );
  usePageTitle("Live Radio");
  const update = (next: LiveSearch) => {
    const query = paramsFromSearch(next);
    router.replace(query ? `${pathname}?${query}` : pathname, {
      scroll: false,
    });
  };
  return (
    <div className="relative mx-auto w-full max-w-310 overflow-x-clip px-5 pb-36 pt-6 text-zinc-50 sm:px-8">
      <LiveRadioHeader active="browse" title="Live Radio">
        <SearchBar onChange={update} search={search} />
      </LiveRadioHeader>
      {hasFilters(search) ? <Results search={search} /> : <Landing />}
    </div>
  );
}
