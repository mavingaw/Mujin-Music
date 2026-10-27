"use client";

import { Button } from "@/components/ui/button";
import { Import } from "lucide-react";
import Link from "next/link";
import ServiceConnections from "./service-connections";

/** Settings tab: which music services are connected and what each may do. */
export default function ConnectionsSettings() {
  return (
    <div className="grid gap-6">
      <section>
        <div className="flex flex-wrap items-start gap-3">
          <div className="min-w-0 flex-1">
            <h2 className="text-base font-semibold text-white">
              Music services
            </h2>
            <p className="mt-1 max-w-2xl text-sm leading-6 text-zinc-500">
              Connect a service to import its playlists into Mujin Music,
              matched to music you own. Each card lists exactly what that
              service&apos;s terms allow. Mujin Music never streams audio from
              these services, and you can disconnect at any time.
            </p>
          </div>
          <Button asChild variant="outline">
            <Link href="/imports">
              <Import /> Import playlists
            </Link>
          </Button>
        </div>
      </section>
      <ServiceConnections showFile />
    </div>
  );
}
