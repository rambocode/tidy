// Canvas particle field styled after the "Twin Galaxy Rings" effect: a tilted
// two-armed spiral galaxy of thousands of tiny drifting particles that slowly
// rotates and breathes around a procedurally shaded planet at its core. Four
// modes — "idle" rotating galaxy, "scan" fast spin (a scan is running),
// "absorb" continuous intake (particles spiral into the core and respawn at
// the rim, for long-running work), "reclaimed" spiral-in-and-dissipate (the
// post-clean result effect) — and per-view color palettes.
//
// Cost contract: no per-particle gradients or color strings. Every particle
// is a pre-rendered glow sprite drawn with drawImage + globalAlpha, and the
// starfield is painted once into an offscreen layer.

/** "idle" rotates, "scan" spins fast, "absorb" recycles inward, "reclaimed" ends. */
export type ParticleMode = "idle" | "scan" | "absorb" | "reclaimed";

/** Per-view color scheme: ember = clean, azure = optimize, gold = analyze. */
export type ParticlePalette = "ember" | "azure" | "gold";

/** Hue ranges and tints backing one palette. */
interface Palette {
  hueMin: number;
  hueSpan: number;
  /** Starfield tint as "r, g, b". */
  star: string;
  /** Center heat-bloom colors (inner, mid) as "r, g, b". */
  bloom: [string, string];
  /** Planet: equirectangular texture (public/), fallback base colour while
   * it loads, and atmosphere tint "r, g, b". */
  planet: { src: string; base: string; atmo: string };
}

const PALETTES: Record<ParticlePalette, Palette> = {
  // Deep crimson through ember orange (350°–380° wraps to 20°).
  // Textures are NASA public-domain cylindrical maps (Blue Marble, Cassini
  // Jupiter PIA07782, Viking MDIM 2.1 Mars), downscaled to 1024×512.
  ember: {
    hueMin: 350, hueSpan: 30, star: "255, 180, 150", bloom: ["255, 110, 60", "190, 40, 25"],
    planet: { src: "/planet-earth.jpg", base: "#1b4f9a", atmo: "120, 180, 255" },
  },
  azure: {
    hueMin: 195, hueSpan: 45, star: "180, 200, 255", bloom: ["100, 180, 255", "60, 120, 220"],
    planet: { src: "/planet-mars.jpg", base: "#9a5a3a", atmo: "230, 170, 130" },
  },
  gold: {
    hueMin: 28, hueSpan: 26, star: "255, 220, 170", bloom: ["255, 190, 80", "220, 130, 30"],
    planet: { src: "/planet-jupiter.jpg", base: "#c9a06a", atmo: "255, 200, 130" },
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

/** Galaxy geometry constants. */
const ARMS = 2;
/** Logarithmic spiral winding: theta = ARM_TWIST * ln(1 + r * ARM_SCALE). */
const ARM_TWIST = 7;
const ARM_SCALE = 8;
/** Ellipse aspect (viewed at a tilt) and in-plane rotation of the disc. */
const TILT = 0.46;
const DISC_ROT = -0.22;
/** Planet radius as a fraction of the galaxy's major axis. */
const PLANET_R = 0.13;
/** Inner hollow: the rings start just outside the planet. */
const CORE_HOLE = 0.19;
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

/** Build the glow sprites for a palette: a white-hot core fading through the
 * palette hue to transparent. Index 0 is near-white, the rest span the hue
 * range — so drawing needs no gradients or colour strings per particle. */
function buildSprites(pal: Palette): HTMLCanvasElement[] {
  const out: HTMLCanvasElement[] = [];
  for (let i = 0; i < SPRITE_COUNT; i++) {
    const c = document.createElement("canvas");
    c.width = SPRITE_PX;
    c.height = SPRITE_PX;
    const g = c.getContext("2d")!;
    const hue = pal.hueMin + (pal.hueSpan * i) / (SPRITE_COUNT - 1);
    const white = i === 0;
    const half = SPRITE_PX / 2;
    const grad = g.createRadialGradient(half, half, 0, half, half, half);
    grad.addColorStop(0, white ? "rgba(255,255,255,1)" : `hsla(${hue}, 100%, 88%, 1)`);
    grad.addColorStop(0.18, white ? "rgba(255,255,255,0.9)" : `hsla(${hue}, 95%, 68%, 0.9)`);
    grad.addColorStop(0.45, white ? "rgba(255,255,255,0.22)" : `hsla(${hue}, 90%, 50%, 0.28)`);
    grad.addColorStop(1, `hsla(${hue}, 90%, 40%, 0)`);
    g.fillStyle = grad;
    g.fillRect(0, 0, SPRITE_PX, SPRITE_PX);
    out.push(c);
  }
  return out;
}

/** Decoded planet texture: 32-bit pixels (same byte order as ImageData,
 * so they copy straight into an output buffer) plus dimensions. */
interface PlanetTex {
  px: Uint32Array;
  w: number;
  h: number;
}

/** Load a planet texture and decode it to raw pixels for sphere sampling. */
function loadPlanetTex(src: string, onReady: (tex: PlanetTex) => void): void {
  const img = new Image();
  img.onload = () => {
    const c = document.createElement("canvas");
    c.width = img.naturalWidth;
    c.height = img.naturalHeight;
    const g = c.getContext("2d")!;
    g.drawImage(img, 0, 0);
    const data = g.getImageData(0, 0, c.width, c.height).data;
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
): void {
  const canvas = document.createElement("canvas");
  canvas.className = "particles";
  host.prepend(canvas);
  const ctx = canvas.getContext("2d");
  if (!ctx) return;

  const pal = PALETTES[palette];
  const sprites = buildSprites(pal);
  /** Decoded planet texture; null until the image has loaded. */
  let planetTex: PlanetTex | null = null;
  /** Sphere lookup for the current disc size (rebuilt when the size changes). */
  let sphereMap: SphereMap | null = null;
  loadPlanetTex(pal.planet.src, (tex) => {
    planetTex = tex;
    sphereMap = null;
    if (reducedMotion) draw();
  });
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
  let stars: Star[] = [];
  let particles: Particle[] = [];
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
    const n = Math.floor((W * H) / 3500);
    for (let i = 0; i < n; i++) {
      stars.push({
        x: Math.random() * W,
        y: Math.random() * H,
        r: Math.random() * 1.2 + 0.3,
        a: Math.random() * 0.5 + 0.15,
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
    // Arm scatter widens with radius so the outer rings look loose while
    // the inner rings stay crisp. Gaussian-ish (sum of two uniforms) so the
    // arm is a soft band with a bright spine, not a hard-edged thread.
    const scatter = (Math.random() + Math.random() - 1) * (0.3 + r * 0.6);
    p.r = r;
    p.theta = (arm * Math.PI * 2) / ARMS + ARM_TWIST * Math.log(1 + r * ARM_SCALE) + scatter;
    p.vr = 0;
    p.vt = 0;
    p.z = (Math.random() - 0.5) * 6 * (1 - r * 0.5);
    // Sprite half-size in px; a few larger sparks bloom above the dust.
    p.size = Math.random() < 0.88 ? Math.random() * 2.2 + 2 : Math.random() * 3 + 4;
    p.alpha = 0.7 + Math.random() * 0.3;
    // ~35% near-white sparks (sprite 0), the rest spread over the hue range.
    p.sprite = Math.random() < 0.35 ? 0 : 1 + Math.floor(Math.random() * (SPRITE_COUNT - 1));
    p.phase = Math.random() * Math.PI * 2;
    p.life = 1;
  };

  /** Build the galaxy: density falls off toward the rim, hollow at the core. */
  const initParticles = () => {
    particles = [];
    // Density scales with the host area (small boxes like the analyze
    // sidebar host only a few hundred) and is capped for cost.
    const count = Math.max(160, Math.min(1600, Math.floor((W * H) / 480)));
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
    shade.addColorStop(1, "rgba(0,0,0,0.72)");
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

  /** Paint stars, particles, and (in reclaimed/absorb mode) the center bloom. */
  const draw = () => {
    ctx.clearRect(0, 0, W, H);
    ctx.globalCompositeOperation = "source-over";
    ctx.globalAlpha = 1;
    ctx.drawImage(starLayer, 0, 0, W, H);

    const t = performance.now() * 0.001;
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
    // so the rings visibly pass behind and in front of the planet. Plain
    // source-over blending: additive compositing is costly in WKWebView and
    // the sprites already carry their glow.
    const pass = (far: boolean) => {
      for (let i = 0; i < n; i++) {
        const p = particles[i];
        if (p.life <= 0.01) continue;
        const y = proj[i * 2 + 1];
        if ((y < cy) !== far) continue;
        const x = proj[i * 2];
        const tw = 0.75 + Math.sin(t * 2.2 + p.phase) * 0.25;
        const depth = far ? 0.8 : 1;
        ctx.globalAlpha = Math.min(1, p.alpha * p.life * tw * (1.25 - p.r * 0.5) * depth);
        const s = p.size;
        ctx.drawImage(sprites[p.sprite], x - s, y - s, s * 2, s * 2);
      }
      ctx.globalAlpha = 1;
    };
    pass(true);
    drawPlanet();
    pass(false);

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

  /** Advance one physics step (rotate, breathe, or spiral in by mode). */
  const update = () => {
    const t = performance.now() * 0.001;
    // Slow breathing swell shared by the whole disc; faster while scanning.
    const swell = mode === "scan" ? 1 + Math.sin(t * 2.2) * 0.03 : 1 + Math.sin(t * 0.6) * 0.035;
    R = baseR * swell;
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
    return;
  }
  loop();
}
