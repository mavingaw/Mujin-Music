import { Suspense } from "react";
import GenreDetails from "./genre-details";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";

export const metadata: Metadata = {
  title: "Genre",
  description: "A genre, its sub-genres and the music in your library.",
};

export default function GenrePage() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <GenreDetails />
    </Suspense>
  );
}
