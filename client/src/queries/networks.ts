import { useQuery } from "@tanstack/react-query";

export interface NetworkInfo {
  name: string;
  addresses: string[];
  status: string;
}

export interface NetworksResponse {
  interfaces: NetworkInfo[];
  configured_networks: string[];
}

export function useNetworks() {
  return useQuery<NetworksResponse>({
    queryKey: ["networks"],
    queryFn: async () => {
      const res = await fetch("/api/v1/networks");
      if (!res.ok) throw new Error("Failed to fetch networks");
      return res.json();
    },
  });
}
