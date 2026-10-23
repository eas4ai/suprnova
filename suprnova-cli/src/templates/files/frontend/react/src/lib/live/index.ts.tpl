import { router, usePage } from "@inertiajs/react";
import { useEffect, useRef } from "react";
import {
  browserLiveTransport,
  startInertiaLive,
  type InertiaLivePage,
  type InertiaLiveRouter,
} from "./core";

/** Connect this mounted page to the Live topics rendered by the server. */
export function useLive(): void {
  const page = usePage();
  const current = useRef(page);
  current.current = page;

  useEffect(() => {
    const transport = browserLiveTransport();
    if (transport === null) return;
    const connection = startInertiaLive({
      getPage: () => current.current as unknown as InertiaLivePage,
      router: router as unknown as InertiaLiveRouter,
      transport,
    });
    return connection.close;
  }, []);
}
