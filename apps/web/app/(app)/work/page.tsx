import { Suspense } from "react";
import WorkDetails from "./work-details";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";

export const metadata: Metadata = {
  title: "Work",
  description: "A composition and every recording of it in your library.",
};

export default function WorkPage() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <WorkDetails />
    </Suspense>
  );
}
