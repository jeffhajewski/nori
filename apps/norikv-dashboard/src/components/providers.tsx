"use client";

import { useEffect, useState, type ReactNode } from "react";
import { wsClient } from "@/lib/wsClient";
import { mockEventGenerator } from "@/lib/mockEvents";

interface ProvidersProps {
  children: ReactNode;
}

export function Providers({ children }: ProvidersProps) {
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);

    // Check if we should use mock data (no vizd available)
    const useMock = process.env.NEXT_PUBLIC_USE_MOCK === "true";

    if (useMock) {
      console.log("[Providers] Starting mock event generator");
      mockEventGenerator.start();
    } else {
      console.log("[Providers] Connecting to vizd");
      wsClient.connect();
    }

    return () => {
      if (useMock) {
        mockEventGenerator.stop();
      } else {
        wsClient.disconnect();
      }
    };
  }, []);

  // Prevent hydration mismatch
  if (!mounted) {
    return null;
  }

  return <>{children}</>;
}
