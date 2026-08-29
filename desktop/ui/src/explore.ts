/** One real object returned from the app-owned SQLite celestial catalog. The
 * map coordinates are derived from catalog RA/Dec and are used only for the
 * two-dimensional explorer projection. */
export interface CelestialTarget {
  id: string;
  name: string;
  bodyType: "star" | "planet" | "dwarf_planet" | "moon" | "exoplanet";
  mapX: number;
  mapY: number;
  distanceLy: number | null;
  radiusKm: number | null;
  discoveryYear: number | null;
  discoveryMethod: string | null;
  hostName: string | null;
  summaryZh: string;
  summaryEn: string;
  sourceUrl: string;
  /** 0..100 catalog importance used for level-of-detail selection. */
  prominence: number;
}

/** Catalog provenance and refresh status. `archiveComplete=false` means the
 * app is showing the offline Solar System seed because no archive snapshot
 * has been downloaded successfully yet. */
export interface CelestialCatalog {
  objects: CelestialTarget[];
  updatedAt: number | null;
  archiveComplete: boolean;
  sourceName: string;
  sourceUrl: string;
  warning: string | null;
}

/** Discovery progress sent from the renderer to the clean-page UI. */
export interface CelestialDiscovery {
  target: CelestialTarget;
  discovered: number;
  total: number;
  isNew: boolean;
}
