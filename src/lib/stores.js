import { writable } from "svelte/store";
export const jobs = writable({});         // { [clusterName]: Job[] }
export const connections = writable({});  // { [clusterName]: { status, detail, next_retry_secs } }
export const config = writable(null);     // full config object from backend
export const viewMode = writable("expanded"); // "expanded" | "compact"
export const uiState = writable({ view_mode: "expanded", collapsed_clusters: [], open_health: [], open_projects: [], open_efficiency: [], open_disks: [] });
export const history = writable([]);
// { [clusterName]: ProjectInfo[] } — filled as projects panels are opened.
export const projectsByCluster = writable({});
