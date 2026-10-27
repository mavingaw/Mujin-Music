import { Suspense } from "react";
import type { Metadata } from "next";
import RouteLoading from "@/components/app/route-loading";
import MyLiveRadio from "@/features/live-radio/my-live-radio";

export const metadata: Metadata = {
  title: "My Live Radio",
  description: "Your saved live radio stations.",
};

export default function MyLiveRadioPage() {
  return (
    <Suspense fallback={<RouteLoading />}>
      <MyLiveRadio />
    </Suspense>
  );
}
