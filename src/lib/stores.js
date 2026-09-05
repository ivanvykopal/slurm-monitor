import { writable } from "svelte/store";
export const jobs = writable({});         // { [clusterName]: Job[] }
export const connections = writable({});  // { [clusterName]: { status, detail, next_retry_secs } }
export const config = writable(null);     // full config object from backend
export const viewMode = writable("expanded"); // "expanded" | "compact"
export const uiState = writable({ view_mode: "expanded", collapsed_clusters: [], open_health: [], open_projects: [] });
export const history = writable([]);
