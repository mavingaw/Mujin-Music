"use client";

import { invalidateCatalogRevisionQueries } from "@/features/library/library-readiness-state";
import { getApiErrorMessage } from "@/features/library/tools/tool-errors";
import { getToolJobs, type ToolJob } from "@parson/music-sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef } from "react";
import { toast } from "sonner";
import { finishedJobs } from "./tools-model";

export const toolJobsKey = ["library-tools", "jobs"] as const;

/** Every running and recent job; polls every second while one runs. */
export function useToolJobs() {
  return useQuery({
    queryKey: toolJobsKey,
    queryFn: getToolJobs,
    refetchInterval: (query) =>
      query.state.data?.some((job) => job.state === "running") ? 1000 : false,
    refetchOnWindowFocus: true,
  });
}

/**
 * Watches jobs from one place: when a job finishes, says so and refreshes
 * every library-tools view and the catalog.
 */
export function useToolJobWatcher() {
  const queryClient = useQueryClient();
  const jobs = useToolJobs();
  const previous = useRef<ToolJob[] | null>(null);
  useEffect(() => {
    const next = jobs.data;
    if (!next) return;
    const before = previous.current;
    previous.current = next;
    if (!before) return;
    const done = finishedJobs(before, next);
    if (!done.length) return;
    for (const job of done) {
      if (job.state === "failed") toast(job.message || `${job.title} failed.`);
      else if (job.state === "cancelled") toast(`${job.title} stopped.`);
      else toast.success(job.message || `${job.title} finished.`);
    }
    void queryClient.invalidateQueries({ queryKey: ["library-tools"] });
    void invalidateCatalogRevisionQueries(queryClient);
  }, [jobs.data, queryClient]);
  return jobs;
}

/** Starts a job and makes the job list poll straight away. */
export function useStartToolJob() {
  const queryClient = useQueryClient();
  return useCallback(
    async (start: () => Promise<ToolJob>, failure: string) => {
      try {
        const job = await start();
        queryClient.setQueryData<ToolJob[]>(toolJobsKey, (current) => [
          job,
          ...(current ?? []).filter((item) => item.id !== job.id),
        ]);
        await queryClient.invalidateQueries({ queryKey: toolJobsKey });
        return job;
      } catch (error) {
        toast(getApiErrorMessage(error, failure));
        await queryClient.invalidateQueries({ queryKey: toolJobsKey });
        return null;
      }
    },
    [queryClient],
  );
}
