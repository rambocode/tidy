// Canvas space field: a tilted four-armed galaxy of fine stellar dust, broad
// translucent nebula clouds, and a naturally coloured planet at its core. It
// slowly rotates and breathes without reading as a pair of neon orbit rings.
// Four modes — "idle" rotating galaxy, "scan" fast spin (a scan is running),
// "absorb" continuous intake (particles spiral into the core and respawn at
// the rim, for long-running work), "reclaimed" spiral-in-and-dissipate (the
// post-clean result effect) — and per-view color palettes.
//
// Cost contract: no per-particle gradients or color strings. Every particle
// is a pre-rendered glow sprite drawn with drawImage + globalAlpha, and the
// starfield is painted once into an offscreen layer.

import type { CelestialDiscovery, CelestialTarget } from "./explore";

/** "idle" rotates, "scan" spins fast, "absorb" recycles inward, "reclaimed" ends. */
export type ParticleMode = "idle" | "scan" | "absorb" | "reclaimed";

/** Per-view celestial scene: ember = Earth, azure = Mars, gold = Jupiter. */
export type ParticlePalette = "ember" | "azure" | "gold";

/** Optional interaction contract used only by the clean-page explorer. */
export interface ParticleOptions {
  interactive?: boolean;
  targets?: readonly CelestialTarget[];
  ariaLabel?: string;
  onDiscovery?: (event: CelestialDiscovery) => void;
  onExit?: () => void;
}

export type CelestialPreset = "all" | "solar-system" | "earth-moon";

/** Small public control surface; ordinary particle hosts can ignore it. */
export interface ParticleController {
  setExploring(active: boolean): void;
  setTargets(targets: readonly CelestialTarget[]): void;
  focusPreset(preset: CelestialPreset): void;
  resetView(): void;
}

/** Celestial colours backing one page scene. */
interface Palette {
  /** Stable seed keeps each page's cloud structure consistent across mounts. */
  seed: number;
  /** Two restrained cloud tints as "r, g, b"; stars stay mostly neutral. */
  nebula: [string, string];
  /** Background stars use a cool-white base independent of the page tint. */
  star: string;
  /** Center heat-bloom colors (inner, mid) as "r, g, b". */
  bloom: [string, string];
  /** Planet: equirectangular texture (public/), fallback base colour while
   * it loads, atmosphere tint "r, g, b", texture shadow curve, optional
   * ocean lift, and the opacity of the dark limb shading. */
  planet: {
    src: string;
    base: string;
    atmo: string;
    gamma: number;
    /** Per-channel texture balance keeps each photographed body recognisable. */
    balance: [number, number, number];
    oceanLift: number;
    limbShade: number;
  };
}

const PALETTES: Record<ParticlePalette, Palette> = {
  // Textures are NASA public-domain cylindrical maps (Blue Marble, Cassini
  // Jupiter PIA07782, Viking MDIM 2.1 Mars), downscaled to 1024×512.
  ember: {
    seed: 17,
    nebula: ["104, 142, 220", "128, 101, 181"],
    star: "218, 230, 255",
    bloom: ["117, 176, 255", "82, 113, 196"],
    // The source texture has near-black oceans. Lift its shadows and soften
    // the limb so the Earth remains legible against the dark clean-page sky.
    planet: {
      src: "/planet-earth.jpg",
      base: "#377fd0",
      atmo: "142, 194, 255",
      gamma: 0.78,
      balance: [1, 1, 1],
      oceanLift: 0.42,
      limbShade: 0.48,
    },
  },
  azure: {
    seed: 29,
    nebula: ["182, 104, 78", "103, 126, 184"],
    star: "224, 231, 246",
    bloom: ["235, 137, 91", "130, 95, 112"],
    planet: {
      src: "/planet-mars.jpg",
      base: "#b96a45",
      atmo: "255, 168, 116",
      gamma: 0.84,
      balance: [1.16, 0.94, 0.82],
      oceanLift: 0,
      limbShade: 0.56,
    },
  },
  gold: {
    seed: 43,
    nebula: ["202, 160, 104", "131, 111, 178"],
    star: "235, 232, 222",
    bloom: ["245, 195, 123", "158, 116, 92"],
    planet: {
      src: "/planet-jupiter.jpg",
      base: "#d1aa78",
      atmo: "255, 216, 170",
      gamma: 0.88,
      balance: [1.05, 1.02, 0.92],
      oceanLift: 0,
      limbShade: 0.54,
    },
  },
};

/** One galaxy particle, kept in polar galaxy space (r normalized 0..1). */
interface Particle {
  /** Normalized orbit radius: 0 = core, 1 = rim. */
  r: number;
  /** Orbit angle in galaxy space (radians). */
  theta: number;
  /** Radial velocity (absorb/reclaimed) in normalized units per frame. */
  vr: number;
  /** Extra angular speed (absorb/reclaimed spiral). */
  vt: number;
  /** Vertical thickness offset in px (the disc is not perfectly flat). */
  z: number;
  size: number;
  alpha: number;
  /** Index into the sprite atlas (which glow colour this dot uses). */
  sprite: number;
  /** Twinkle phase. */
  phase: number;
  life: number;
}

/** One background star (static position, twinkling alpha). */
interface Star {
  x: number;
  y: number;
  r: number;
  a: number;
  tw: number;
}

/** Mutable hit-test state kept separate from the immutable target catalog. */
interface TargetRuntime {
  target: CelestialTarget;
  discovered: boolean;
  visible: boolean;
  x: number;
  y: number;
  depth: number;
  hitRadius: number;
}

/** Galaxy geometry constants. */
const ARMS = 4;
/** Logarithmic spiral winding: theta = ARM_TWIST * ln(1 + r * ARM_SCALE). */
const ARM_TWIST = 3.8;
const ARM_SCALE = 6;
/** Ellipse aspect (viewed at a tilt) and in-plane rotation of the disc. */
const TILT = 0.46;
const DISC_ROT = -0.22;
/** Planet radius as a fraction of the galaxy's major axis. */
const PLANET_R = 0.13;
/** Inner hollow: stellar dust starts just outside the planet. */
const CORE_HOLE = 0.16;
/** Per-pixel sphere lookup for one disc size: which output pixels are
 * inside the disc, their texture row offset, and their longitude fraction. */
interface SphereMap {
  side: number;
  idx: Int32Array;
  rowBase: Int32Array;
  lonFrac: Float32Array;
  out: ImageData;
}

/** Build the orthographic lookup for a disc of `side` device pixels: each
 * pixel inside the disc gets its latitude row in the texture and its
 * longitude (as a fraction of a full turn) on the visible hemisphere. Done
 * once per size; per frame only the longitude scroll is added. */
function buildSphereMap(side: number, tex: PlanetTex): SphereMap {
  const half = side / 2;
  const idx: number[] = [];
  const rows: number[] = [];
  const lons: number[] = [];
  for (let y = 0; y < side; y++) {
    const dy = (y + 0.5 - half) / half;
    for (let x = 0; x < side; x++) {
      const dx = (x + 0.5 - half) / half;
      if (dx * dx + dy * dy > 1) continue;
      const lat = Math.asin(dy);
      const cosl = Math.cos(lat);
      const lon = Math.asin(Math.max(-1, Math.min(1, dx / Math.max(cosl, 1e-4))));
      idx.push(y * side + x);
      rows.push(Math.min(tex.h - 1, Math.floor((lat / Math.PI + 0.5) * tex.h)) * tex.w);
      lons.push(lon / (Math.PI * 2));
    }
  }
  return {
    side,
    idx: Int32Array.from(idx),
    rowBase: Int32Array.from(rows),
    lonFrac: Float32Array.from(lons),
    out: new ImageData(side, side),
  };
}
/** Sprite atlas: pre-rendered glow dots, SPRITE_PX square each. */
const SPRITE_PX = 32;
const SPRITE_COUNT = 6;
const NEBULA_PX = 512;

/** Build a restrained stellar sprite set. Most particles are white, blue-white,
 * or warm-white; only two sprites borrow the scene's cloud colours. */
function buildSprites(pal: Palette): HTMLCanvasElement[] {
  const out: HTMLCanvasElement[] = [];
  const colours = [
    "255, 255, 255",
    "208, 225, 255",
    "165, 196, 244",
    "255, 225, 190",
    pal.nebula[0],
    pal.nebula[1],
  ];
  for (let i = 0; i < SPRITE_COUNT; i++) {
    const c = document.createElement("canvas");
    c.width = SPRITE_PX;
    c.height = SPRITE_PX;
    const g = c.getContext("2d")!;
    const colour = colours[i];
    const half = SPRITE_PX / 2;
    const grad = g.createRadialGradient(half, half, 0, half, half, half);
    grad.addColorStop(0, `rgba(${colour}, 1)`);
    grad.addColorStop(0.16, `rgba(${colour}, 0.8)`);
    grad.addColorStop(0.42, `rgba(${colour}, 0.18)`);
    grad.addColorStop(1, `rgba(${colour}, 0)`);
    g.fillStyle = grad;
    g.fillRect(0, 0, SPRITE_PX, SPRITE_PX);
    out.push(c);
  }
  return out;
}

/** Small deterministic PRNG used only while pre-rendering the nebula layer.
 * Stable clouds avoid a visual jump when a view remounts or resizes. */
function seededRandom(seed: number): () => number {
  let value = seed >>> 0;
  return () => {
    value += 0x6d2b79f5;
    let next = value;
    next = Math.imul(next ^ (next >>> 15), next | 1);
    next ^= next + Math.imul(next ^ (next >>> 7), next | 61);
    return ((next ^ (next >>> 14)) >>> 0) / 4294967296;
  };
}

/** Pre-render broad, low-contrast cloud clusters along the same logarithmic
 * arms as the stars. The finished layer is transformed once per frame; no
 * gradients are allocated inside the animation loop. */
function buildNebulaLayer(pal: Palette): HTMLCanvasElement {
  const layer = document.createElement("canvas");
  layer.width = NEBULA_PX;
  layer.height = NEBULA_PX;
  const g = layer.getContext("2d")!;
  const half = NEBULA_PX / 2;
  const random = seededRandom(pal.seed);

  // A restrained milky core ties the separate cloud clusters together.
  const core = g.createRadialGradient(half, half, half * 0.08, half, half, half * 0.82);
  core.addColorStop(0, `rgba(${pal.nebula[0]}, 0.16)`);
  core.addColorStop(0.34, `rgba(${pal.nebula[1]}, 0.075)`);
  core.addColorStop(1, `rgba(${pal.nebula[1]}, 0)`);
  g.fillStyle = core;
  g.fillRect(0, 0, NEBULA_PX, NEBULA_PX);

  for (let i = 0; i < 110; i++) {
    const r = CORE_HOLE + Math.pow(random(), 0.76) * (0.96 - CORE_HOLE);
    const arm = i % ARMS;
    const scatter = (random() + random() + random() - 1.5) * (0.2 + r * 0.5);
    const theta = (arm * Math.PI * 2) / ARMS + ARM_TWIST * Math.log(1 + r * ARM_SCALE) + scatter;
    const x = half + Math.cos(theta) * r * half * 0.94;
    const y = half + Math.sin(theta) * r * half * 0.94;
    const radius = (12 + random() * 24) * (0.72 + r * 0.5);
    const colour = pal.nebula[i % pal.nebula.length];
    const alpha = 0.025 + random() * 0.045;

    g.save();
    g.translate(x, y);
    g.rotate(theta + Math.PI / 2);
    g.scale(1.8 + random() * 0.8, 0.62 + random() * 0.28);
    const cloud = g.createRadialGradient(0, 0, 0, 0, 0, radius);
    cloud.addColorStop(0, `rgba(${colour}, ${alpha})`);
    cloud.addColorStop(0.48, `rgba(${colour}, ${alpha * 0.52})`);
    cloud.addColorStop(1, `rgba(${colour}, 0)`);
    g.fillStyle = cloud;
    g.fillRect(-radius, -radius, radius * 2, radius * 2);
    g.restore();

    // Sparse dark lanes break the glow into dusty, irregular arm segments.
    if (i % 4 === 0) {
      g.save();
      g.translate(
        x + Math.cos(theta + 0.25) * radius * 0.45,
        y + Math.sin(theta + 0.25) * radius * 0.45,
      );
      g.rotate(theta + Math.PI / 2);
      g.scale(2.1, 0.42);
      const lane = g.createRadialGradient(0, 0, 0, 0, 0, radius * 0.8);
      lane.addColorStop(0, "rgba(7, 10, 24, 0.07)");
      lane.addColorStop(1, "rgba(7, 10, 24, 0)");
      g.fillStyle = lane;
      g.fillRect(-radius, -radius, radius * 2, radius * 2);
      g.restore();
    }
  }
  return layer;
}

/** Decoded planet texture: 32-bit pixels (same byte order as ImageData,
 * so they copy straight into an output buffer) plus dimensions. */
interface PlanetTex {
  px: Uint32Array;
  w: number;
  h: number;
}

/** Load a planet texture and decode it to colour-graded raw pixels for sphere
 * sampling. Gamma lifts the whole texture's shadows. `oceanLift` additionally
 * blends blue-dominant pixels toward a clearer ocean blue, without washing
 * out green and brown land pixels. */
function loadPlanetTex(
  src: string,
  gamma: number,
  balance: [number, number, number],
  oceanLift: number,
  onReady: (tex: PlanetTex) => void,
): void {
  const img = new Image();
  img.onload = () => {
    const c = document.createElement("canvas");
    c.width = img.naturalWidth;
    c.height = img.naturalHeight;
    const g = c.getContext("2d")!;
    g.drawImage(img, 0, 0);
    const data = g.getImageData(0, 0, c.width, c.height).data;
    for (let i = 0; i < data.length; i += 4) {
      const red = data[i];
      const green = data[i + 1];
      const blue = data[i + 2];
      const liftedRed = Math.pow(red / 255, gamma) * 255 * balance[0];
      const liftedGreen = Math.pow(green / 255, gamma) * 255 * balance[1];
      const liftedBlue = Math.pow(blue / 255, gamma) * 255 * balance[2];
      // Blue dominance separates oceans from land and snow. Scaling the
      // blend by that dominance keeps coastlines natural instead of painting
      // a uniform blue veil over the complete globe.
      const blueDominance = Math.max(0, Math.min(1, (blue - Math.max(red, green) * 1.08) / 52));
      const oceanBlend = blueDominance * oceanLift;
      data[i] = liftedRed + (45 - liftedRed) * oceanBlend;
      data[i + 1] = liftedGreen + (125 - liftedGreen) * oceanBlend;
      data[i + 2] = liftedBlue + (215 - liftedBlue) * oceanBlend;
    }
    onReady({ px: new Uint32Array(data.buffer), w: c.width, h: c.height });
  };
  img.src = src;
}

/** Mount the particle canvas into a positioned container and run it until
 * the canvas leaves the DOM (view remount replaces innerHTML, stopping it). */
export function mountParticles(
  host: HTMLElement,
  mode: ParticleMode = "idle",
  palette: ParticlePalette = "ember",
  options: ParticleOptions = {},
): ParticleController {
  const canvas = document.createElement("canvas");
  canvas.className = "particles";
  host.prepend(canvas);
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    return {
      setExploring: () => {},
      setTargets: () => {},
      focusPreset: () => {},
      resetView: () => {},
    };
  }

  const pal = PALETTES[palette];
  const sprites = buildSprites(pal);
  const nebulaLayer = buildNebulaLayer(pal);
  /** Decoded planet texture; null until the image has loaded. */
  let planetTex: PlanetTex | null = null;
  /** Sphere lookup for the current disc size (rebuilt when the size changes). */
  let sphereMap: SphereMap | null = null;
  loadPlanetTex(
    pal.planet.src,
    pal.planet.gamma,
    pal.planet.balance,
    pal.planet.oceanLift,
    (tex) => {
      planetTex = tex;
      sphereMap = null;
      if (reducedMotion) draw();
    },
  );
  // 1.5× is visually indistinguishable for soft glows and saves ~45% fill.
  const dpr = Math.min(window.devicePixelRatio || 1, 1.5);
  /** Static starfield, painted once per resize and blitted every frame. */
  const starLayer = document.createElement("canvas");
  let W = 0;
  let H = 0;
  let cx = 0;
  let cy = 0;
  /** Galaxy rim radius in px along the major axis (R breathes; baseR does not). */
  let R = 0;
  let baseR = 0;
  let viewScale = 1;
  let stars: Star[] = [];
  let particles: Particle[] = [];
  let targets: TargetRuntime[] = [];
  const replaceTargets = (catalog: readonly CelestialTarget[]) => {
    const discovered = new Set(targets.filter((target) => target.discovered).map((target) => target.target.id));
    targets = catalog.map((target) => ({
      target,
      discovered: discovered.has(target.id),
      visible: false,
      x: 0,
      y: 0,
      depth: 1,
      hitRadius: 0,
    }));
    keyboardTargetIndex = Math.min(keyboardTargetIndex, Math.max(0, targets.length - 1));
    hoveredTarget = null;
  };
  let exploring = false;
  let hoveredTarget: TargetRuntime | null = null;
  let keyboardTargetIndex = 0;
  let dragging = false;
  let dragMoved = 0;
  let lastPointerX = 0;
  let lastPointerY = 0;
  /** Catalog camera center in wrapped map coordinates. Deep zoom progressively
   * reveals lower-prominence objects around this visual center. */
  let cameraX = 0;
  let cameraY = 0;
  let activePreset: CelestialPreset = "all";
  replaceTargets(options.targets ?? []);
  // Absorption progress only advances in "reclaimed" mode; it drives the
  // growing center bloom that remains after every particle has dissipated.
  let absorbed = 0;
  // Global disc rotation (the planet's surface scroll derives from it).
  let spin = 0;
  /** Last frame timestamp for dt-based motion (dropped frames do not slow
   * the rotation, so the eye reads a steady speed). */
  let last = performance.now();
  /** Projected screen positions for this frame (x, y interleaved). */
  let proj = new Float32Array(0);
  /** Cached planet overlays (halo + shading), rebuilt on resize. */
  const haloLayer = document.createElement("canvas");
  const shadeLayer = document.createElement("canvas");
  /** Per-frame sphere scratch: strips render here unclipped, then the
   * finished square is drawn once under a circular clip so the limb gets a
   * single antialiased edge instead of one per strip. */
  const sphereLayer = document.createElement("canvas");

  const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  /** Size the canvas to the host (dpr-scaled) and rebuild the starfield. */
  const resize = () => {
    W = host.clientWidth;
    H = host.clientHeight;
    canvas.width = Math.max(1, Math.floor(W * dpr));
    canvas.height = Math.max(1, Math.floor(H * dpr));
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    cx = W / 2;
    // The planet sits in the upper-middle; hero copy lives below the disc.
    cy = H * 0.4;
    // The disc fills the width; its vertical extent is capped so the whole
    // galaxy stays above the hero copy (bottom ~28% of the host).
    baseR = Math.min(W * 0.44, (H * 0.3) / TILT);
    R = baseR;
    stars = [];
    const n = Math.floor((W * H) / 2400);
    for (let i = 0; i < n; i++) {
      stars.push({
        x: Math.random() * W,
        y: Math.random() * H,
        r:
          Math.random() < 0.92
            ? Math.random() * 0.65 + 0.18
            : Math.random() * 1.1 + 0.65,
        a: Math.random() * 0.38 + 0.08,
        tw: Math.random() * Math.PI * 2,
      });
    }
    paintStars();
    buildPlanetLayers();
    if (particles.length === 0) initParticles();
  };

  /** Paint the starfield once into its own layer (static, no twinkle). */
  const paintStars = () => {
    starLayer.width = canvas.width;
    starLayer.height = canvas.height;
    const g = starLayer.getContext("2d")!;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    for (const st of stars) {
      g.beginPath();
      g.fillStyle = `rgba(${pal.star}, ${st.a})`;
      g.arc(st.x, st.y, st.r, 0, Math.PI * 2);
      g.fill();
    }
  };

  /** Place one particle on a spiral arm at radius r (normalized). */
  const seed = (p: Particle, r: number) => {
    const arm = Math.floor(Math.random() * ARMS);
    // Most dust follows a broad spiral arm; some fills the inter-arm space.
    // Three summed uniforms produce a soft density spine without a hard edge.
    const followsArm = Math.random() < 0.78;
    const scatter = (Math.random() + Math.random() + Math.random() - 1.5) * (0.24 + r * 0.9);
    p.r = r;
    p.theta = followsArm
      ? (arm * Math.PI * 2) / ARMS + ARM_TWIST * Math.log(1 + r * ARM_SCALE) + scatter
      : Math.random() * Math.PI * 2;
    p.vr = 0;
    p.vt = 0;
    p.z = (Math.random() - 0.5) * 12 * (1 - r * 0.35);
    // Most stars are sub-2px dust. A few larger points provide depth without
    // turning the arms into evenly sized beads.
    p.size =
      Math.random() < 0.94
        ? Math.random() * 0.8 + 0.45
        : Math.random() * 1.5 + 1.2;
    p.alpha = followsArm ? 0.46 + Math.random() * 0.4 : 0.18 + Math.random() * 0.3;
    const colourRoll = Math.random();
    if (colourRoll < 0.44) p.sprite = 0;
    else if (colourRoll < 0.66) p.sprite = 1;
    else if (colourRoll < 0.8) p.sprite = 2;
    else if (colourRoll < 0.91) p.sprite = 3;
    else p.sprite = 4 + Math.floor(Math.random() * 2);
    p.phase = Math.random() * Math.PI * 2;
    p.life = 1;
  };

  /** Build the galaxy: density falls off toward the rim, hollow at the core. */
  const initParticles = () => {
    particles = [];
    // Density scales with the host area (small boxes like the analyze
    // sidebar host only a few hundred) and is capped for cost.
    const count = Math.max(180, Math.min(2800, Math.floor((W * H) / 340)));
    for (let i = 0; i < count; i++) {
      const p = {} as Particle;
      seed(p, CORE_HOLE + Math.pow(Math.random(), 0.9) * (1 - CORE_HOLE));
      particles.push(p);
    }
  };

  /** Project a particle from galaxy space to canvas pixels. */
  const project = (p: Particle): [number, number] => {
    const a = p.theta + spin;
    const gx = Math.cos(a) * p.r * R;
    const gy = Math.sin(a) * p.r * R * TILT + p.z;
    // Rotate the tilted disc in-plane around the fixed planet center.
    const x = gx * Math.cos(DISC_ROT) - gy * Math.sin(DISC_ROT);
    const y = gx * Math.sin(DISC_ROT) + gy * Math.cos(DISC_ROT);
    return [cx + x, cy + y];
  };

  /** Pre-render the planet's atmosphere halo and sphere shading at the
   * current size; both are static so they cost one drawImage per frame. */
  const buildPlanetLayers = () => {
    const pr = Math.max(1, Math.floor(R * PLANET_R));
    const pad = Math.ceil(pr * 1.4);
    haloLayer.width = pad * 2 * dpr;
    haloLayer.height = pad * 2 * dpr;
    const hg = haloLayer.getContext("2d")!;
    hg.setTransform(dpr, 0, 0, dpr, 0, 0);
    const halo = hg.createRadialGradient(pad, pad, pr * 0.9, pad, pad, pr * 1.35);
    halo.addColorStop(0, `rgba(${pal.planet.atmo}, 0.35)`);
    halo.addColorStop(1, `rgba(${pal.planet.atmo}, 0)`);
    hg.fillStyle = halo;
    hg.fillRect(0, 0, pad * 2, pad * 2);

    shadeLayer.width = pr * 2 * dpr;
    shadeLayer.height = pr * 2 * dpr;
    const sg = shadeLayer.getContext("2d")!;
    sg.setTransform(dpr, 0, 0, dpr, 0, 0);
    // Sphere shading: bright toward the upper-left light, dark at the rim,
    // clipped to the disc so it can be drawn without a clip each frame.
    sg.beginPath();
    sg.arc(pr, pr, pr, 0, Math.PI * 2);
    sg.clip();
    const shade = sg.createRadialGradient(pr * 0.55, pr * 0.55, pr * 0.1, pr, pr, pr);
    shade.addColorStop(0, "rgba(255,255,255,0.22)");
    shade.addColorStop(0.55, "rgba(0,0,0,0.05)");
    shade.addColorStop(1, `rgba(0,0,0,${pal.planet.limbShade})`);
    sg.fillStyle = shade;
    sg.fillRect(0, 0, pr * 2, pr * 2);
  };

  /** Paint the planet: cached halo, sphere-mapped texture strips clipped to
   * a disc, then the cached shading overlay. */
  const drawPlanet = () => {
    const pr = R * PLANET_R;
    if (pr < 4) return;
    const pad = pr * 1.4;
    ctx.drawImage(haloLayer, cx - pad, cy - pad, pad * 2, pad * 2);

    ctx.save();
    ctx.beginPath();
    ctx.arc(cx, cy, pr, 0, Math.PI * 2);
    ctx.clip();
    if (!planetTex) {
      ctx.fillStyle = pal.planet.base;
      ctx.fillRect(cx - pr, cy - pr, pr * 2, pr * 2);
    } else {
      // Disc size is taken from the unswelled radius so the lookup is not
      // rebuilt every frame; the breathing swell is a sub-pixel scale.
      const side = Math.ceil(baseR * PLANET_R * 2 * dpr);
      if (!sphereMap || sphereMap.side !== side) {
        sphereMap = buildSphereMap(side, planetTex);
        sphereLayer.width = side;
        sphereLayer.height = side;
      }
      // Per-pixel orthographic sampling: longitude = stored offset + scroll.
      const scroll = (((spin * 0.35) % 1) + 1) % 1;
      const { idx, rowBase, lonFrac, out } = sphereMap;
      const dst = new Uint32Array(out.data.buffer);
      const tw = planetTex.w;
      const tp = planetTex.px;
      for (let i = 0; i < idx.length; i++) {
        let u = lonFrac[i] + scroll;
        u -= Math.floor(u);
        dst[idx[i]] = tp[rowBase[i] + ((u * tw) | 0)];
      }
      sphereLayer.getContext("2d")!.putImageData(out, 0, 0);
      ctx.drawImage(sphereLayer, cx - pr, cy - pr, pr * 2, pr * 2);
    }
    ctx.restore();
    ctx.drawImage(shadeLayer, cx - pr, cy - pr, pr * 2, pr * 2);
  };

  /** Project the pre-rendered circular cloud field into the same tilted,
   * rotating plane as the stellar arms. It stays behind every star and the
   * planet, acting as depth rather than a foreground colour wash. */
  const drawNebula = (time: number) => {
    const scale = R / (NEBULA_PX / 2);
    ctx.save();
    ctx.translate(cx, cy);
    ctx.rotate(DISC_ROT);
    ctx.scale(scale, scale * TILT);
    ctx.rotate(spin);
    ctx.drawImage(nebulaLayer, -NEBULA_PX / 2, -NEBULA_PX / 2);

    // Two inexpensive live gradients drift across the pre-rendered clouds.
    // Their low alpha adds changing volume without rebuilding the 110-cloud
    // nebula layer or allocating a gradient for every cloud each frame.
    const liveAlpha = (exploring ? 0.052 : 0.032) * (0.86 + Math.sin(time * 0.58) * 0.14);
    for (let i = 0; i < 2; i++) {
      const phase = time * (exploring ? 0.22 : 0.12) + i * Math.PI;
      const x = Math.cos(phase) * NEBULA_PX * 0.16;
      const y = Math.sin(phase * 0.73) * NEBULA_PX * 0.1;
      const radius = NEBULA_PX * (0.31 + i * 0.05);
      const glow = ctx.createRadialGradient(x, y, 0, x, y, radius);
      glow.addColorStop(0, `rgba(${pal.nebula[i]}, ${liveAlpha})`);
      glow.addColorStop(0.46, `rgba(${pal.nebula[i]}, ${liveAlpha * 0.45})`);
      glow.addColorStop(1, `rgba(${pal.nebula[i]}, 0)`);
      ctx.fillStyle = glow;
      ctx.fillRect(-NEBULA_PX / 2, -NEBULA_PX / 2, NEBULA_PX, NEBULA_PX);
    }
    ctx.restore();
  };

  /** Keep right ascension in the wrapped -1/1 map range. */
  const wrappedCameraX = (value: number): number => {
    let wrapped = value;
    while (wrapped > 1) wrapped -= 2;
    while (wrapped < -1) wrapped += 2;
    return wrapped;
  };

  interface SkyProjection {
    x: number;
    y: number;
    /** 0 = far side, 1 = nearest point on the celestial sphere. */
    depth: number;
    front: boolean;
  }

  /** Project catalog RA/Dec-derived map coordinates through a rotatable
   * celestial sphere. Yaw follows horizontal drag; pitch follows vertical
   * drag, and perspective makes the near hemisphere visibly larger. */
  const projectSky = (mapX: number, mapY: number): SkyProjection => {
    const longitude = mapX * Math.PI;
    const latitude = -mapY * (Math.PI / 2);
    const relativeLongitude = longitude - cameraX * Math.PI;
    const cameraPitch = -cameraY * (Math.PI / 2);
    const cosLatitude = Math.cos(latitude);
    const sphereX = Math.sin(relativeLongitude) * cosLatitude;
    const sphereY = Math.sin(latitude);
    const sphereZ = Math.cos(relativeLongitude) * cosLatitude;
    const rotatedY = sphereY * Math.cos(cameraPitch) - sphereZ * Math.sin(cameraPitch);
    const rotatedZ = sphereY * Math.sin(cameraPitch) + sphereZ * Math.cos(cameraPitch);
    const depth = (rotatedZ + 1) / 2;
    const perspective = 0.58 + depth * 0.42;
    const scale = baseR * viewScale;
    return {
      x: cx + sphereX * scale * perspective,
      y: cy - rotatedY * scale * perspective,
      depth,
      front: rotatedZ > -0.12,
    };
  };

  /** Subtle longitude/latitude lines make the third dimension legible during
   * drag. They are an instrument overlay, not decorative orbit rings. */
  const drawSkyGrid = () => {
    if (!exploring) return;
    const alpha = Math.max(0.018, 0.1 / Math.sqrt(viewScale));
    const drawSamples = (
      count: number,
      coordinateAt: (index: number) => [number, number],
    ) => {
      let drawing = false;
      ctx.beginPath();
      for (let index = 0; index <= count; index++) {
        const [mapX, mapY] = coordinateAt(index);
        const point = projectSky(mapX, mapY);
        if (!point.front) {
          drawing = false;
          continue;
        }
        if (drawing) ctx.lineTo(point.x, point.y);
        else ctx.moveTo(point.x, point.y);
        drawing = true;
      }
      ctx.stroke();
    };
    ctx.save();
    ctx.lineWidth = 0.7;
    ctx.strokeStyle = `rgba(174, 207, 255, ${alpha})`;
    for (const mapY of [-0.66, -0.33, 0, 0.33, 0.66]) {
      drawSamples(96, (index) => [-1 + (index / 96) * 2, mapY]);
    }
    for (const mapX of [-0.75, -0.5, -0.25, 0, 0.25, 0.5, 0.75, 1]) {
      drawSamples(64, (index) => [mapX, -1 + (index / 64) * 2]);
    }
    ctx.restore();
  };

  /** Convert physical radius to a visible radius without pretending the huge
   * star/planet range fits linearly on one screen. The logarithm preserves
   * correct size ordering; the hit ring remains larger for accessibility. */
  const physicalMarkerRadius = (radiusKm: number | null): number => {
    if (radiusKm === null || radiusKm <= 0) return 2.2;
    const earthRadii = radiusKm / 6_371;
    return Math.max(1.6, Math.min(11, 2.4 + Math.log10(earthRadii) * 2.2));
  };

  const targetSprite = (target: CelestialTarget): number => {
    if (target.bodyType === "star") return 3;
    if (target.bodyType === "moon" || target.bodyType === "dwarf_planet") return 2;
    let hash = 0;
    for (const char of target.id) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
    return 1 + (hash % (SPRITE_COUNT - 1));
  };

  /** Level-of-detail threshold. Each zoom step lowers the prominence gate, so
   * continuing to zoom reveals every catalog row near the visual center. */
  const minimumProminence = (): number => {
    if (viewScale < 2) return 90;
    if (viewScale < 4) return 62;
    if (viewScale < 8) return 42;
    if (viewScale < 16) return 24;
    return 0;
  };

  const belongsToPreset = (target: CelestialTarget): boolean => {
    if (activePreset === "all") return true;
    if (activePreset === "earth-moon") return target.id === "earth" || target.id === "moon";
    return !target.id.includes(":");
  };

  /** Draw visible catalog objects and update their screen-space hit regions.
   * The renderer receives the complete SQLite snapshot but culls by viewport
   * and level of detail before issuing any canvas draw calls. */
  const drawTargets = (time: number) => {
    if (!exploring) return;
    const minProminence = minimumProminence();
    for (const runtime of targets) {
      const { target } = runtime;
      const point = projectSky(target.mapX, target.mapY);
      runtime.x = point.x;
      runtime.y = point.y;
      runtime.depth = point.depth;
      runtime.visible =
        belongsToPreset(target) &&
        target.prominence >= minProminence &&
        point.front &&
        runtime.x > -24 &&
        runtime.x < W + 24 &&
        runtime.y > -24 &&
        runtime.y < H + 24;
      if (!runtime.visible) continue;

      // Zoom grows the body itself as well as the spacing. A logarithmic gain
      // keeps deep zoom useful without letting a stellar marker fill the frame.
      const zoomGain = 1 + Math.log2(Math.max(1, viewScale)) * 1.05;
      const depthGain = 0.68 + point.depth * 0.32;
      const markerRadius = Math.min(48, physicalMarkerRadius(target.radiusKm) * zoomGain * depthGain);
      runtime.hitRadius = Math.max(9, markerRadius + 5);
      const phase = target.mapX * 8.3 + target.mapY * 5.7;
      const glowRadius = (markerRadius + 2.4) * (1 + Math.sin(time * 1.8 + phase) * 0.06);
      ctx.globalAlpha = (runtime.discovered ? 0.82 : 1) * (0.48 + point.depth * 0.52);
      ctx.drawImage(
        sprites[targetSprite(target)],
        runtime.x - glowRadius,
        runtime.y - glowRadius,
        glowRadius * 2,
        glowRadius * 2,
      );

      const hovered = hoveredTarget === runtime;
      const pulse = 0.72 + Math.sin(time * 2.4 + phase) * 0.18;
      ctx.beginPath();
      ctx.arc(runtime.x, runtime.y, runtime.hitRadius + (hovered ? 3 : 0), 0, Math.PI * 2);
      ctx.lineWidth = hovered ? 1.5 : 1;
      ctx.strokeStyle = runtime.discovered
        ? `rgba(83, 214, 215, ${hovered ? 0.95 : 0.5})`
        : `rgba(228, 238, 255, ${hovered ? 0.95 : pulse * 0.58})`;
      ctx.stroke();

      if (hovered) {
        ctx.globalAlpha = 0.9;
        ctx.fillStyle = "rgba(238, 245, 255, 0.96)";
        ctx.font = '11px -apple-system, "SF Pro Text", sans-serif';
        ctx.textAlign = "center";
        ctx.fillText(target.name, runtime.x, runtime.y - runtime.hitRadius - 8);
      }
    }
    ctx.globalAlpha = 1;
  };

  /** Paint background stars, nebula, galaxy dust, planet, and task bloom. */
  const draw = () => {
    const t = performance.now() * 0.001;
    ctx.clearRect(0, 0, W, H);
    ctx.globalCompositeOperation = "source-over";
    ctx.globalAlpha = 1;
    ctx.drawImage(starLayer, 0, 0, W, H);
    drawNebula(t);
    drawSkyGrid();
    // Project every particle exactly once per frame into a flat buffer.
    const n = particles.length;
    if (proj.length < n * 2) proj = new Float32Array(n * 2);
    for (let i = 0; i < n; i++) {
      const [x, y] = project(particles[i]);
      proj[i * 2] = x;
      proj[i * 2 + 1] = y;
    }
    // Two passes split at the planet's equator: the far half of the disc
    // (above center) is painted first, then the planet, then the near half —
    // so the stellar dust visibly passes behind and in front of the planet.
    // Plain source-over blending: additive compositing is costly in WKWebView
    // and the sprites already carry their glow.
    const pass = (far: boolean) => {
      for (let i = 0; i < n; i++) {
        const p = particles[i];
        if (p.life <= 0.01) continue;
        const y = proj[i * 2 + 1];
        if ((y < cy) !== far) continue;
        const x = proj[i * 2];
        const tw = 0.9 + Math.sin(t * 1.7 + p.phase) * 0.1;
        const depth = far ? 0.72 : 1;
        ctx.globalAlpha = Math.min(1, p.alpha * p.life * tw * (1.18 - p.r * 0.28) * depth);
        const s = p.size;
        ctx.drawImage(sprites[p.sprite], x - s, y - s, s * 2, s * 2);
      }
      ctx.globalAlpha = 1;
    };
    pass(true);
    // Normal pages keep the textured decorative planet. Exploration replaces
    // it with catalog markers so every object's size follows the same physical
    // logarithmic scale instead of giving Earth a privileged fake diameter.
    if (!exploring) drawPlanet();
    pass(false);
    drawTargets(t);

    // Reclaimed/absorb: a heat bloom at the core — growing as the galaxy
    // dissipates (reclaimed) or held steady while the intake runs (absorb).
    if ((mode === "reclaimed" || mode === "absorb") && absorbed > 0) {
      const intensity = 0.14 + absorbed * 0.32;
      const r = Math.min(120, Math.min(W, H) * 0.4);
      const cg = ctx.createRadialGradient(cx, cy, 0, cx, cy, r);
      cg.addColorStop(0, `rgba(${pal.bloom[0]}, ${intensity})`);
      cg.addColorStop(0.6, `rgba(${pal.bloom[1]}, ${intensity * 0.3})`);
      cg.addColorStop(1, "rgba(0,0,0,0)");
      ctx.fillStyle = cg;
      ctx.fillRect(cx - r, cy - r, r * 2, r * 2);
    }
    ctx.globalCompositeOperation = "source-over";
  };

  /** Nearest target under a CSS-pixel canvas coordinate. */
  const targetAt = (x: number, y: number): TargetRuntime | null => {
    let nearest: TargetRuntime | null = null;
    let nearestDistance = Number.POSITIVE_INFINITY;
    for (const runtime of targets) {
      if (!runtime.visible) continue;
      const distance = Math.hypot(x - runtime.x, y - runtime.y);
      if (distance <= runtime.hitRadius && distance < nearestDistance) {
        nearest = runtime;
        nearestDistance = distance;
      }
    }
    return nearest;
  };

  /** Mark a target as found and report stable progress to the host UI. */
  const discover = (runtime: TargetRuntime) => {
    const isNew = !runtime.discovered;
    runtime.discovered = true;
    hoveredTarget = runtime;
    options.onDiscovery?.({
      target: runtime.target,
      discovered: targets.filter((target) => target.discovered).length,
      total: targets.length,
      isNew,
    });
  };

  const pointerPosition = (event: PointerEvent): [number, number] => {
    const rect = canvas.getBoundingClientRect();
    return [event.clientX - rect.left, event.clientY - rect.top];
  };

  const updatePointerCursor = () => {
    canvas.style.cursor = dragging ? "grabbing" : hoveredTarget ? "pointer" : "grab";
  };

  const onPointerDown = (event: PointerEvent) => {
    if (!exploring) return;
    dragging = true;
    dragMoved = 0;
    lastPointerX = event.clientX;
    lastPointerY = event.clientY;
    canvas.setPointerCapture(event.pointerId);
    updatePointerCursor();
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!exploring) return;
    if (dragging) {
      const deltaX = event.clientX - lastPointerX;
      const deltaY = event.clientY - lastPointerY;
      const scale = Math.max(1, baseR * viewScale);
      cameraX = wrappedCameraX(cameraX - deltaX / (scale * Math.PI));
      cameraY = Math.max(
        -0.92,
        Math.min(0.92, cameraY - deltaY / (scale * (Math.PI / 2))),
      );
      dragMoved += Math.hypot(deltaX, deltaY);
      lastPointerX = event.clientX;
      lastPointerY = event.clientY;
      if (reducedMotion) draw();
      return;
    }
    const [x, y] = pointerPosition(event);
    hoveredTarget = targetAt(x, y);
    if (hoveredTarget) keyboardTargetIndex = targets.indexOf(hoveredTarget);
    updatePointerCursor();
    if (reducedMotion) draw();
  };

  const onPointerUp = (event: PointerEvent) => {
    if (!exploring) return;
    const [x, y] = pointerPosition(event);
    if (dragMoved < 6) {
      const target = targetAt(x, y);
      if (target) discover(target);
    }
    dragging = false;
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    hoveredTarget = targetAt(x, y);
    updatePointerCursor();
    if (reducedMotion) draw();
  };

  const onPointerCancel = (event: PointerEvent) => {
    dragging = false;
    dragMoved = 0;
    if (canvas.hasPointerCapture(event.pointerId)) canvas.releasePointerCapture(event.pointerId);
    updatePointerCursor();
  };

  const onWheel = (event: WheelEvent) => {
    if (!exploring) return;
    event.preventDefault();
    const zoom = event.deltaY < 0 ? 1.14 : 0.88;
    viewScale = Math.max(0.8, Math.min(64, viewScale * zoom));
    if (reducedMotion) {
      R = baseR * viewScale;
      draw();
    }
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (!exploring) return;
    if (event.key === "Escape") {
      options.onExit?.();
      event.preventDefault();
    } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      cameraX = wrappedCameraX(
        cameraX + (event.key === "ArrowLeft" ? -0.08 : 0.08) / viewScale,
      );
      event.preventDefault();
    } else if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      if (targets.length > 0) {
        const direction = event.key === "ArrowUp" ? -1 : 1;
        keyboardTargetIndex =
          (keyboardTargetIndex + direction + targets.length) % targets.length;
        hoveredTarget = targets[keyboardTargetIndex];
      }
      event.preventDefault();
    } else if (event.key === "+" || event.key === "=") {
      viewScale = Math.min(64, viewScale * 1.14);
      event.preventDefault();
    } else if (event.key === "-" || event.key === "_") {
      viewScale = Math.max(0.8, viewScale * 0.88);
      event.preventDefault();
    } else if (event.key === "Enter" || event.key === " ") {
      const target = hoveredTarget ?? targets[keyboardTargetIndex];
      if (target) discover(target);
      event.preventDefault();
    } else if (event.key === "0" || event.key === "1" || event.key === "2") {
      focusPreset(event.key === "1" ? "solar-system" : event.key === "2" ? "earth-moon" : "all");
      event.preventDefault();
    }
    if (reducedMotion) {
      R = baseR * viewScale;
      draw();
    }
  };

  if (options.interactive) {
    canvas.setAttribute("role", "application");
    canvas.setAttribute("aria-label", options.ariaLabel ?? "Celestial explorer");
    canvas.addEventListener("pointerdown", onPointerDown);
    canvas.addEventListener("pointermove", onPointerMove);
    canvas.addEventListener("pointerup", onPointerUp);
    canvas.addEventListener("pointercancel", onPointerCancel);
    canvas.addEventListener("wheel", onWheel, { passive: false });
    canvas.addEventListener("keydown", onKeyDown);
  }

  const focusPreset = (preset: CelestialPreset) => {
    activePreset = preset;
    hoveredTarget = null;
    if (preset === "solar-system") {
      cameraX = 0.115;
      cameraY = 0;
      viewScale = 1.35;
    } else if (preset === "earth-moon") {
      cameraX = 0.009;
      cameraY = -0.007;
      viewScale = 6;
    } else {
      cameraX = 0;
      cameraY = 0;
      viewScale = 1;
    }
    R = baseR * viewScale;
    if (reducedMotion) draw();
  };

  const controller: ParticleController = {
    setExploring(active) {
      if (!options.interactive) return;
      exploring = active;
      hoveredTarget = null;
      dragging = false;
      canvas.style.pointerEvents = active ? "auto" : "none";
      canvas.tabIndex = active ? 0 : -1;
      canvas.style.cursor = active ? "grab" : "default";
      if (active) canvas.focus({ preventScroll: true });
      else {
        viewScale = 1;
        R = baseR;
        cameraX = 0;
        cameraY = 0;
        activePreset = "all";
      }
      if (reducedMotion) draw();
    },
    setTargets(catalog) {
      replaceTargets(catalog);
      if (reducedMotion) draw();
    },
    focusPreset,
    resetView() {
      spin = 0;
      focusPreset("all");
    },
  };

  /** Advance one physics step (rotate, breathe, or spiral in by mode). */
  const update = () => {
    const t = performance.now() * 0.001;
    // Slow breathing swell shared by the whole disc; faster while scanning.
    const swell = mode === "scan" ? 1 + Math.sin(t * 2.2) * 0.03 : 1 + Math.sin(t * 0.6) * 0.035;
    R = baseR * viewScale * swell;
    // Global rotation; the scan mode spins noticeably faster.
    // Angular speed in rad/s, integrated over real elapsed time (clamped so
    // a background tab does not jump when it wakes).
    const now = performance.now();
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    const omega = mode === "scan" ? 0.72 : mode === "idle" ? 0.13 : 0.36;
    spin += omega * dt;

    if (mode === "idle" || mode === "scan") return;

    for (const p of particles) {
      if (p.life <= 0) {
        // Absorb keeps the intake endless: dissolved particles re-enter at
        // the rim instead of staying dead like in reclaimed mode.
        if (mode === "absorb") seed(p, 0.85 + Math.random() * 0.15);
        else continue;
      }
      // Spiral inward: radial pull grows near the core, with an extra
      // angular kick so the arms wind up as they fall in.
      p.vr -= 0.00035 / Math.max(p.r, 0.08);
      p.vr = Math.max(p.vr, -0.02);
      p.vt += 0.0006;
      p.r += p.vr;
      p.theta += p.vt;
      if (p.r < PLANET_R) {
        p.life -= 0.08;
        p.size *= 0.96;
      }
    }

    if (mode === "reclaimed" && particles.length > 0) {
      const alive = particles.filter((p) => p.life > 0.05).length;
      absorbed = 1 - alive / particles.length;
    } else if (mode === "absorb") {
      // Steady mid-strength bloom: the intake never "finishes" while running.
      absorbed = 0.35;
    }
  };

  /** rAF loop; exits (and releases the observer) once the canvas leaves
   * the DOM — view remounts replace innerHTML, which detaches it. */
  const loop = () => {
    if (!canvas.isConnected) {
      observer.disconnect();
      return;
    }
    update();
    draw();
    requestAnimationFrame(loop);
  };

  const observer = new ResizeObserver(() => {
    if (!canvas.isConnected) {
      observer.disconnect();
      return;
    }
    resize();
    if (reducedMotion) draw();
  });
  observer.observe(host);

  resize();
  if (reducedMotion) {
    // Reduced motion: one static galaxy frame (plus final bloom when
    // reclaimed), no animation loop at all.
    if (mode === "reclaimed" || mode === "absorb") {
      particles = [];
      absorbed = 1;
    }
    draw();
    return controller;
  }
  loop();
  return controller;
}
