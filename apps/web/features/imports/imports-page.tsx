"use client";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  IMPORT_FILE_ACCEPT,
  MAX_PLAYLIST_FILE_BYTES,
  completeSpotifyAuthorization,
  createImportJob,
  decodePlaylistFile,
  getImportJob,
  getImportJobs,
  getRemotePlaylists,
  importErrorMessage,
  importMatchRate,
  parseImportUrl,
  type CreateImportJob,
  type ImportJobSummary,
  type ImportService,
  type RemotePlaylist,
} from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  ArrowLeft,
  ChevronRight,
  FileUp,
  Heart,
  Link2,
  ListMusic,
  Loader2,
  Plug,
  TriangleAlert,
} from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import {
  useEffect,
  useRef,
  useState,
  type DragEvent,
  type FormEvent,
} from "react";
import { toast } from "sonner";
import CapabilityChips from "./capability-chips";
import ImportReview from "./import-review";
import {
  SERVICE_ACCENTS,
  SERVICE_SHORT_NAMES,
  jobProgressLabel,
  percent,
} from "./import-review-state";
import ServiceConnections, {
  IMPORT_SERVICES_QUERY_KEY,
  useImportServices,
} from "./service-connections";

const JOBS_QUERY_KEY = ["import-jobs"] as const;

function inFlight(job: Pick<ImportJobSummary, "status">) {
  return job.status === "fetching" || job.status === "matching";
}

function useStartImport() {
  const router = useRouter();
  const queryClient = useQueryClient();
  const [pending, setPending] = useState(false);
  async function start(source: CreateImportJob) {
    if (pending) return;
    setPending(true);
    try {
      const job = await createImportJob(source);
      void queryClient.invalidateQueries({ queryKey: JOBS_QUERY_KEY });
      router.push(`/imports?job=${encodeURIComponent(job.id)}`);
    } catch (error) {
      toast.error(importErrorMessage(error, "The import did not start."));
    } finally {
      setPending(false);
    }
  }
  return { start, pending };
}

function SpotifyCallback({ code, state }: { code: string; state: string }) {
  const router = useRouter();
  const queryClient = useQueryClient();
  const handled = useRef(false);
  useEffect(() => {
    if (handled.current) return;
    handled.current = true;
    completeSpotifyAuthorization(code, state)
      .then(() => {
        toast.success("Spotify connected. Your playlists are below.");
        void queryClient.invalidateQueries({
          queryKey: IMPORT_SERVICES_QUERY_KEY,
        });
      })
      .catch((error) =>
        toast.error(importErrorMessage(error, "Spotify was not connected.")),
      )
      .finally(() => router.replace("/imports"));
  }, [code, queryClient, router, state]);
  return (
    <div className="flex items-center gap-2 py-20 text-sm text-zinc-400">
      <Loader2 className="h-4 w-4 animate-spin" /> Finishing Spotify sign-in…
    </div>
  );
}

function LinkImport({
  pending,
  onImport,
}: {
  pending: boolean;
  onImport: (source: CreateImportJob) => void;
}) {
  const [value, setValue] = useState("");
  const parsed = parseImportUrl(value);
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (parsed) onImport({ url: value.trim() });
  };
  return (
    <form
      className="rounded-xl border border-white/8 bg-white/2.5 p-4"
      onSubmit={submit}
    >
      <label
        className="flex items-center gap-2 text-sm font-semibold text-white"
        htmlFor="import-link"
      >
        <Link2 className="h-4 w-4 text-zinc-400" /> Paste a playlist link
      </label>
      <p className="mt-1 text-xs text-zinc-500">
        Spotify, Apple Music, YouTube Music or Deezer.
      </p>
      <div className="mt-3 flex gap-2">
        <Input
          id="import-link"
          onChange={(event) => setValue(event.target.value)}
          placeholder="https://open.spotify.com/playlist/…"
          value={value}
        />
        <Button disabled={!parsed || pending} type="submit">
          {pending && <Loader2 className="animate-spin" />}
          Import
        </Button>
      </div>
      <p aria-live="polite" className="mt-2 h-4 text-xs">
        {value.trim() &&
          (parsed ? (
            <span className="inline-flex items-center gap-1.5 text-zinc-400">
              <span
                className={`h-1.5 w-1.5 rounded-full ${SERVICE_ACCENTS[parsed.service]}`}
              />
              {SERVICE_SHORT_NAMES[parsed.service]}{" "}
              {parsed.kind === "saved_tracks"
                ? "Liked Songs"
                : parsed.kind.replace("_", " ")}
            </span>
          ) : (
            <span className="text-zinc-600">
              Not a supported playlist link.
            </span>
          ))}
      </p>
    </form>
  );
}

function FileImport({
  pending,
  onImport,
}: {
  pending: boolean;
  onImport: (source: CreateImportJob) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  async function read(file: File | undefined) {
    if (!file) return;
    if (file.size > MAX_PLAYLIST_FILE_BYTES) {
      toast.error("Playlist files can be at most 5 MB.");
      return;
    }
    const content = decodePlaylistFile(
      new Uint8Array(await file.arrayBuffer()),
    );
    onImport({ file_name: file.name, content });
  }
  const drop = (event: DragEvent) => {
    event.preventDefault();
    setDragging(false);
    void read(event.dataTransfer.files[0]);
  };
  return (
    <div
      className={`rounded-xl border border-dashed p-4 transition-colors ${
        dragging ? "border-white/40 bg-white/6" : "border-white/12 bg-white/2"
      }`}
      onDragLeave={() => setDragging(false)}
      onDragOver={(event) => {
        event.preventDefault();
        setDragging(true);
      }}
      onDrop={drop}
    >
      <p className="flex items-center gap-2 text-sm font-semibold text-white">
        <FileUp className="h-4 w-4 text-zinc-400" /> Upload a playlist file
      </p>
      <p className="mt-1 text-xs leading-5 text-zinc-500">
        M3U, M3U8, PLS, XSPF, CSV/TSV (Exportify, TuneMyMusic, Soundiiz), Apple
        Music text export or iTunes XML. Drop it here or
      </p>
      <input
        accept={IMPORT_FILE_ACCEPT}
        className="hidden"
        onChange={(event) => {
          void read(event.target.files?.[0]);
          event.target.value = "";
        }}
        ref={input}
        type="file"
      />
      <Button
        className="mt-3"
        disabled={pending}
        onClick={() => input.current?.click()}
        variant="outline"
      >
        {pending ? <Loader2 className="animate-spin" /> : <FileUp />}
        Choose file
      </Button>
    </div>
  );
}

function RemotePlaylistCard({
  playlist,
  service,
  disabled,
  onImport,
}: {
  playlist: RemotePlaylist;
  service: ImportService;
  disabled: boolean;
  onImport: () => void;
}) {
  return (
    <li className="flex min-w-0 items-center gap-3 rounded-lg border border-white/6 bg-white/2 p-2.5">
      {playlist.image_url ? (
        // eslint-disable-next-line @next/next/no-img-element
        <img
          alt=""
          className="h-11 w-11 shrink-0 rounded object-cover"
          loading="lazy"
          referrerPolicy="no-referrer"
          src={playlist.image_url}
        />
      ) : (
        <div
          className={`grid h-11 w-11 shrink-0 place-items-center rounded ${
            playlist.kind === "saved_tracks"
              ? "bg-linear-to-br from-indigo-500 to-emerald-400 text-white"
              : "bg-white/5 text-zinc-500"
          }`}
        >
          {playlist.kind === "saved_tracks" ? (
            <Heart className="h-4 w-4 fill-current" />
          ) : (
            <ListMusic className="h-4 w-4" />
          )}
        </div>
      )}
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm text-zinc-100">{playlist.name}</p>
        <p className="truncate text-xs text-zinc-500">
          {[
            playlist.track_count != null
              ? `${playlist.track_count} tracks`
              : null,
            playlist.owner,
          ]
            .filter(Boolean)
            .join(" · ") || service.name}
        </p>
      </div>
      <Button
        disabled={disabled}
        onClick={onImport}
        size="sm"
        variant="outline"
      >
        Import
      </Button>
    </li>
  );
}

function ServicePlaylists({
  services,
  pending,
  onImport,
}: {
  services: ImportService[];
  pending: boolean;
  onImport: (source: CreateImportJob) => void;
}) {
  const listable = services.filter(
    (service) =>
      service.connection &&
      (service.id === "spotify" ||
        (service.id === "apple_music" &&
          service.connection.details.has_user_token === true) ||
        (service.id === "deezer" &&
          Boolean(service.connection.details.profile_id))),
  );
  const [selected, setSelected] = useState<string | null>(null);
  const active =
    listable.find((service) => service.id === selected) ?? listable[0];
  const playlists = useQuery({
    queryKey: ["import-remote-playlists", active?.id],
    queryFn: () =>
      getRemotePlaylists(active!.id as Exclude<ImportService["id"], "file">),
    enabled: Boolean(active),
    staleTime: 60_000,
    retry: false,
  });
  if (!active) return null;
  return (
    <section className="grid gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <h2 className="mr-2 text-base font-semibold text-white">
          From your services
        </h2>
        {listable.map((service) => (
          <button
            aria-pressed={service.id === active.id}
            className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium ${
              service.id === active.id
                ? "bg-white text-black"
                : "bg-white/6 text-zinc-300 hover:bg-white/10"
            }`}
            key={service.id}
            onClick={() => setSelected(service.id)}
            type="button"
          >
            <span
              className={`h-1.5 w-1.5 rounded-full ${SERVICE_ACCENTS[service.id]}`}
            />
            {service.name}
          </button>
        ))}
      </div>
      <CapabilityChips capabilities={active.capabilities} compact />
      {playlists.isPending ? (
        <div className="flex items-center gap-2 text-sm text-zinc-500">
          <Loader2 className="h-4 w-4 animate-spin" /> Loading playlists…
        </div>
      ) : playlists.isError ? (
        <p className="flex items-center gap-2 text-sm text-zinc-500">
          <TriangleAlert className="h-4 w-4 text-amber-400" />
          {importErrorMessage(
            playlists.error,
            "Playlists could not be loaded.",
          )}
        </p>
      ) : playlists.data.length === 0 ? (
        <p className="text-sm text-zinc-500">No playlists found.</p>
      ) : (
        <ul className="grid gap-2 sm:grid-cols-2 xl:grid-cols-3">
          {playlists.data.map((playlist) => (
            <RemotePlaylistCard
              disabled={pending}
              key={`${playlist.kind}-${playlist.id}`}
              onImport={() =>
                onImport({
                  service: active.id as Exclude<ImportService["id"], "file">,
                  kind: playlist.kind,
                  id: playlist.id,
                })
              }
              playlist={playlist}
              service={active}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function RecentImports({ jobs }: { jobs: ImportJobSummary[] }) {
  if (!jobs.length) return null;
  return (
    <section className="grid gap-3">
      <h2 className="text-base font-semibold text-white">Recent imports</h2>
      <ul className="grid gap-1.5">
        {jobs.map((job) => {
          const rate = importMatchRate(job);
          return (
            <li key={job.id}>
              <Link
                className="group flex min-w-0 items-center gap-3 rounded-lg border border-white/6 bg-white/2 px-3 py-2.5 hover:bg-white/5"
                href={`/imports?job=${encodeURIComponent(job.id)}`}
              >
                <span
                  aria-hidden
                  className={`h-2 w-2 shrink-0 rounded-full ${SERVICE_ACCENTS[job.service]}`}
                />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm text-zinc-100">
                    {job.name}
                  </span>
                  <span
                    className={`block truncate text-xs ${
                      job.status === "failed"
                        ? "text-rose-300"
                        : "text-zinc-500"
                    }`}
                  >
                    {jobProgressLabel(job)}
                  </span>
                </span>
                {inFlight(job) ? (
                  <Loader2 className="h-4 w-4 shrink-0 animate-spin text-zinc-500" />
                ) : job.status === "ready" ? (
                  <span className="hidden w-28 shrink-0 items-center gap-2 sm:flex">
                    <span className="h-1.5 flex-1 overflow-hidden rounded-full bg-white/8">
                      <span
                        className="block h-full bg-emerald-400"
                        style={{ width: percent(rate) }}
                      />
                    </span>
                    <span className="text-xs tabular-nums text-zinc-500">
                      {percent(rate)}
                    </span>
                  </span>
                ) : null}
                <ChevronRight className="h-4 w-4 shrink-0 text-zinc-600 group-hover:text-white" />
              </Link>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function JobView({ jobId }: { jobId: string }) {
  const router = useRouter();
  const queryClient = useQueryClient();
  const job = useQuery({
    queryKey: ["import-job", jobId],
    queryFn: () => getImportJob(jobId),
    refetchInterval: (query) =>
      query.state.data && inFlight(query.state.data) ? 1_000 : false,
  });
  const back = (
    <Link
      className="inline-flex items-center gap-1 text-sm text-zinc-500 hover:text-white"
      href="/imports"
    >
      <ArrowLeft className="h-4 w-4" /> Imports
    </Link>
  );
  if (job.isPending) {
    return (
      <div className="grid gap-6">
        {back}
        <Loader2 className="h-5 w-5 animate-spin text-zinc-500" />
      </div>
    );
  }
  if (job.isError) {
    return (
      <div className="grid gap-6">
        {back}
        <p className="text-sm text-zinc-500">This import could not be found.</p>
      </div>
    );
  }
  const data = job.data;
  if (data.status !== "ready") {
    return (
      <div className="grid gap-6">
        {back}
        <div className="rounded-xl border border-white/8 bg-white/2.5 p-8 text-center">
          {data.status === "failed" ? (
            <TriangleAlert className="mx-auto h-6 w-6 text-amber-400" />
          ) : (
            <Loader2 className="mx-auto h-6 w-6 animate-spin text-zinc-400" />
          )}
          <h2 className="mt-3 text-lg font-semibold text-white">{data.name}</h2>
          <p className="mt-1 text-sm text-zinc-500">{jobProgressLabel(data)}</p>
          {data.status === "failed" && (
            <Button asChild className="mt-4" variant="outline">
              <Link href="/imports">Back to imports</Link>
            </Button>
          )}
        </div>
      </div>
    );
  }
  return (
    <div className="grid gap-6">
      {back}
      <ImportReview
        job={data}
        key={data.id}
        onDeleted={() => {
          void queryClient.invalidateQueries({ queryKey: JOBS_QUERY_KEY });
          router.replace("/imports");
        }}
        onJobChange={(next) => {
          queryClient.setQueryData(["import-job", jobId], next);
          void queryClient.invalidateQueries({ queryKey: JOBS_QUERY_KEY });
        }}
      />
    </div>
  );
}

function Landing() {
  const services = useImportServices();
  const { start, pending } = useStartImport();
  const [showConnections, setShowConnections] = useState(false);
  const jobs = useQuery({
    queryKey: JOBS_QUERY_KEY,
    queryFn: getImportJobs,
    refetchInterval: (query) =>
      query.state.data?.some(inFlight) ? 1_500 : false,
  });
  const connected = (services.data ?? []).filter(
    (service) => service.connection && service.id !== "file",
  );
  return (
    <div className="grid gap-9">
      <header>
        <h1 className="text-3xl font-semibold text-white">Import playlists</h1>
        <p className="mt-2 max-w-2xl text-sm leading-6 text-zinc-500">
          Bring playlists from Spotify, Apple Music, YouTube Music, Deezer or
          any playlist file. Parson matches every track to music you own and
          shows what it is unsure about. No audio is ever streamed from those
          services.
        </p>
      </header>

      <div className="grid gap-3 md:grid-cols-2">
        <LinkImport
          onImport={(source) => void start(source)}
          pending={pending}
        />
        <FileImport
          onImport={(source) => void start(source)}
          pending={pending}
        />
      </div>

      {services.data && (
        <ServicePlaylists
          onImport={(source) => void start(source)}
          pending={pending}
          services={services.data}
        />
      )}

      <RecentImports jobs={jobs.data ?? []} />

      <section className="grid gap-3">
        <div className="flex flex-wrap items-center gap-3">
          <h2 className="text-base font-semibold text-white">Connections</h2>
          <span className="text-xs text-zinc-500">
            {connected.length
              ? `${connected.map((service) => service.name).join(", ")} connected`
              : "Deezer links and files work without connecting anything."}
          </span>
          <Button
            className="ml-auto"
            onClick={() => setShowConnections((value) => !value)}
            size="sm"
            variant="ghost"
          >
            <Plug /> {showConnections ? "Hide" : "Manage"}
          </Button>
        </div>
        {showConnections && <ServiceConnections />}
      </section>
    </div>
  );
}

export default function ImportsPage() {
  const searchParams = useSearchParams();
  const jobId = searchParams.get("job");
  const code = searchParams.get("code");
  const state = searchParams.get("state");
  const oauthError = searchParams.get("error");
  const router = useRouter();
  useEffect(() => {
    if (oauthError) {
      toast.error(
        oauthError === "access_denied"
          ? "Spotify access was not granted."
          : "Spotify sign-in failed.",
      );
      router.replace("/imports");
    }
  }, [oauthError, router]);

  return (
    <div className="mx-auto w-full max-w-275 px-5 py-9 pb-40 sm:px-7">
      {code && state ? (
        <SpotifyCallback code={code} state={state} />
      ) : jobId ? (
        <JobView jobId={jobId} />
      ) : (
        <Landing />
      )}
    </div>
  );
}
