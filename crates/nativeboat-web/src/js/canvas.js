// The 2D context of <canvas> elements. Scripts keep the drawing state here;
// drawing becomes self-contained commands that the platform runs against
// the canvas bitmap, batched until the bitmap is read or the task ends.
// Paths are stored in canvas pixels: points are transformed when added.

const IDENTITY = [1, 0, 0, 1, 0, 0];
const multiplyMatrix = (m, n) => [
  m[0] * n[0] + m[2] * n[1],
  m[1] * n[0] + m[3] * n[1],
  m[0] * n[2] + m[2] * n[3],
  m[1] * n[2] + m[3] * n[3],
  m[0] * n[4] + m[2] * n[5] + m[4],
  m[1] * n[4] + m[3] * n[5] + m[5],
];
const applyMatrix = (m, x, y) => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];
const invertMatrix = (m) => {
  const det = m[0] * m[3] - m[1] * m[2];
  if (!det) return null;
  return [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det, (m[2] * m[5] - m[3] * m[4]) / det, (m[1] * m[4] - m[0] * m[5]) / det];
};
const finite = (...values) => values.every((value) => Number.isFinite(value));

class DOMMatrix {
  constructor(init = IDENTITY) {
    const [a, b, c, d, e, f] = Array.isArray(init) && init.length === 6 ? init.map(Number) : IDENTITY;
    Object.assign(this, { a, b, c, d, e, f });
  }

  get m11() { return this.a; }
  get m12() { return this.b; }
  get m21() { return this.c; }
  get m22() { return this.d; }
  get m41() { return this.e; }
  get m42() { return this.f; }
  get is2D() { return true; }
  get isIdentity() { return this.toArray().every((value, index) => value === IDENTITY[index]); }

  toArray() {
    return [this.a, this.b, this.c, this.d, this.e, this.f];
  }

  multiply(other) {
    return new DOMMatrix(multiplyMatrix(this.toArray(), matrixOf(other)));
  }

  translate(x = 0, y = 0) {
    return this.multiply({ a: 1, b: 0, c: 0, d: 1, e: x, f: y });
  }

  scale(x = 1, y = x) {
    return this.multiply({ a: x, b: 0, c: 0, d: y, e: 0, f: 0 });
  }

  // SVGMatrix's turn by degrees.
  rotate(degrees = 0) {
    const radians = Number(degrees) * Math.PI / 180;
    const [cos, sin] = [Math.cos(radians), Math.sin(radians)];
    return this.multiply({ a: cos, b: sin, c: -sin, d: cos, e: 0, f: 0 });
  }

  inverse() {
    return new DOMMatrix(invertMatrix(this.toArray()) || [NaN, NaN, NaN, NaN, NaN, NaN]);
  }

  transformPoint(point = {}) {
    const [x, y] = applyMatrix(this.toArray(), point.x || 0, point.y || 0);
    return { x, y, z: 0, w: 1 };
  }
}
const matrixOf = (init) => ['a', 'b', 'c', 'd', 'e', 'f'].map((name, index) => (init && name in init ? Number(init[name]) : IDENTITY[index]));

// Colors are kept as RGBA and serialized like browsers do.
const parseCanvasColor = (text) => {
  const rgba = page.parseColor(String(text));
  return rgba && { r: rgba[0], g: rgba[1], b: rgba[2], a: rgba[3] };
};
const serializeColor = ({ r, g, b, a }) =>
  a >= 1 ? `#${[r, g, b].map((value) => value.toString(16).padStart(2, '0')).join('')}` : `rgba(${r}, ${g}, ${b}, ${a})`;

const gradients = new WeakMap();
class CanvasGradient {
  constructor() {
    illegal();
  }

  addColorStop(offset, color) {
    const position = Number(offset);
    if (!(position >= 0 && position <= 1)) {
      throw new DOMException(`The provided value (${offset}) is outside the range [0.0, 1.0].`, 'IndexSizeError');
    }
    const parsed = parseCanvasColor(color);
    if (!parsed) throw new DOMException(`The value provided ('${color}') could not be parsed as a color.`, 'SyntaxError');
    gradients.get(this).stops.push([position, parsed]);
  }
}
const createGradient = (kind, points) => {
  const gradient = Object.create(CanvasGradient.prototype);
  gradients.set(gradient, { kind, points, stops: [] });
  return gradient;
};

class CanvasPattern {
  constructor() {
    illegal();
  }

  setTransform() {}
}

// The paint of a style, with gradient points mapped by `matrix`.
const paintOf = (style, matrix) => {
  const gradient = gradients.get(style);
  if (gradient) {
    const { kind, points, stops } = gradient;
    if (kind === 'linear') {
      return { Linear: { start: applyMatrix(matrix, points[0], points[1]), end: applyMatrix(matrix, points[2], points[3]), stops } };
    }
    const scale = Math.sqrt(Math.abs(matrix[0] * matrix[3] - matrix[1] * matrix[2]));
    return {
      Radial: {
        start: [...applyMatrix(matrix, points[0], points[1]), points[2] * scale],
        end: [...applyMatrix(matrix, points[3], points[4]), points[5] * scale],
        stops,
      },
    };
  }
  // Patterns are not drawn.
  return { Color: style instanceof CanvasPattern ? { r: 0, g: 0, b: 0, a: 0 } : style };
};

// Elliptical arcs as cubic curves, in user space.
const arcCurves = (x, y, radiusX, radiusY, rotation, start, sweep) => {
  const count = Math.ceil(Math.abs(sweep) / (Math.PI / 2) - 1e-9);
  const step = sweep / count;
  const k = (4 / 3) * Math.tan(step / 4);
  const [cos, sin] = [Math.cos(rotation), Math.sin(rotation)];
  const point = (ux, uy) => [x + radiusX * ux * cos - radiusY * uy * sin, y + radiusX * ux * sin + radiusY * uy * cos];
  const curves = [];
  for (let index = 0; index < count; index++) {
    const from = start + step * index;
    const to = from + step;
    curves.push([
      ...point(Math.cos(from) - k * Math.sin(from), Math.sin(from) + k * Math.cos(from)),
      ...point(Math.cos(to) + k * Math.sin(to), Math.sin(to) - k * Math.cos(to)),
      ...point(Math.cos(to), Math.sin(to)),
    ]);
  }
  return { start: point(Math.cos(start), Math.sin(start)), curves };
};

// The sweep of an arc, following the canvas rules for full circles.
const arcSweep = (start, end, anticlockwise) => {
  const full = 2 * Math.PI;
  if (!anticlockwise && end - start >= full) return full;
  if (anticlockwise && start - end >= full) return -full;
  const sweep = (((end - start) % full) + full) % full;
  return anticlockwise ? (sweep === 0 ? 0 : sweep - full) : sweep;
};

// Builds paths: the context's current path and Path2D objects. Points are
// mapped by the transform `matrix()` returns when they are added.
class PathBuilder {
  constructor(matrix) {
    this.matrix = matrix;
    this.segments = [];
    this.current = null;
    this.start = null;
  }

  add(segment, point) {
    this.segments.push(segment);
    this.current = point;
  }

  moveTo(x, y) {
    if (!finite(x, y)) return;
    const point = [Number(x), Number(y)];
    this.add({ Move: applyMatrix(this.matrix(), ...point) }, point);
    this.start = point;
  }

  lineTo(x, y) {
    if (!finite(x, y)) return;
    if (!this.current) return this.moveTo(x, y);
    const point = [Number(x), Number(y)];
    this.add({ Line: applyMatrix(this.matrix(), ...point) }, point);
  }

  bezierCurveTo(x1, y1, x2, y2, x, y) {
    if (!finite(x1, y1, x2, y2, x, y)) return;
    if (!this.current) this.moveTo(x1, y1);
    const m = this.matrix();
    this.add({ Cubic: [...applyMatrix(m, x1, y1), ...applyMatrix(m, x2, y2), ...applyMatrix(m, x, y)] }, [Number(x), Number(y)]);
  }

  quadraticCurveTo(cx, cy, x, y) {
    if (!finite(cx, cy, x, y)) return;
    if (!this.current) this.moveTo(cx, cy);
    const [x0, y0] = this.current;
    const third = (from, to) => from + (2 / 3) * (to - from);
    this.bezierCurveTo(third(x0, cx), third(y0, cy), third(x, cx), third(y, cy), x, y);
  }

  ellipse(x, y, radiusX, radiusY, rotation, start, end, anticlockwise = false) {
    if (!finite(x, y, radiusX, radiusY, rotation, start, end)) return;
    if (radiusX < 0 || radiusY < 0) throw new DOMException('The radius provided is negative.', 'IndexSizeError');
    const { start: first, curves } = arcCurves(x, y, radiusX, radiusY, rotation, start, arcSweep(start, end, !!anticlockwise));
    if (this.current) this.lineTo(...first);
    else this.moveTo(...first);
    for (const curve of curves) this.bezierCurveTo(...curve);
  }

  arc(x, y, radius, start, end, anticlockwise = false) {
    this.ellipse(x, y, radius, radius, 0, start, end, anticlockwise);
  }

  arcTo(x1, y1, x2, y2, radius) {
    if (!finite(x1, y1, x2, y2, radius)) return;
    if (radius < 0) throw new DOMException('The radius provided is negative.', 'IndexSizeError');
    if (!this.current) this.moveTo(x1, y1);
    const [x0, y0] = this.current;
    const [ax, ay, bx, by] = [x0 - x1, y0 - y1, x2 - x1, y2 - y1];
    const [lengthA, lengthB] = [Math.hypot(ax, ay), Math.hypot(bx, by)];
    const cross = ax * by - ay * bx;
    if (!lengthA || !lengthB || !radius || Math.abs(cross) < 1e-9) return this.lineTo(x1, y1);
    const [ux, uy, vx, vy] = [ax / lengthA, ay / lengthA, bx / lengthB, by / lengthB];
    const angle = Math.acos(Math.max(-1, Math.min(1, ux * vx + uy * vy)));
    const tangent = radius / Math.tan(angle / 2);
    const [sx, sy, ex, ey] = [x1 + ux * tangent, y1 + uy * tangent, x1 + vx * tangent, y1 + vy * tangent];
    const [bisectX, bisectY] = [ux + vx, uy + vy];
    const bisect = Math.hypot(bisectX, bisectY);
    const distance = radius / Math.sin(angle / 2);
    const [cx, cy] = [x1 + (bisectX / bisect) * distance, y1 + (bisectY / bisect) * distance];
    this.lineTo(sx, sy);
    this.arc(cx, cy, radius, Math.atan2(sy - cy, sx - cx), Math.atan2(ey - cy, ex - cx), cross > 0);
  }

  rect(x, y, width, height) {
    if (!finite(x, y, width, height)) return;
    this.moveTo(x, y);
    this.lineTo(x + width, y);
    this.lineTo(x + width, y + height);
    this.lineTo(x, y + height);
    this.closePath();
  }

  roundRect(x, y, width, height, radii = 0) {
    if (!finite(x, y, width, height)) return;
    const list = (Array.isArray(radii) ? radii : [radii]).map((radius) => (typeof radius === 'object' ? Number(radius.x) : Number(radius)));
    if (list.length < 1 || list.length > 4) throw new RangeError(`${list.length} radii provided. Between one and four radii are necessary.`);
    if (list.some((radius) => radius < 0)) throw new RangeError('Radius value is negative.');
    const [tl, tr, br, bl] = [[0, 0, 0, 0], [0, 1, 0, 1], [0, 1, 2, 1], [0, 1, 2, 3]][list.length - 1].map((index) => list[index]);
    const scale = Math.min(1, Math.abs(width) / (tl + tr || 1), Math.abs(width) / (bl + br || 1), Math.abs(height) / (tl + bl || 1), Math.abs(height) / (tr + br || 1));
    const [a, b, c, d] = [tl, tr, br, bl].map((radius) => radius * scale);
    const quarter = Math.PI / 2;
    this.moveTo(x + a, y);
    this.lineTo(x + width - b, y);
    this.arc(x + width - b, y + b, b, -quarter, 0);
    this.lineTo(x + width, y + height - c);
    this.arc(x + width - c, y + height - c, c, 0, quarter);
    this.lineTo(x + d, y + height);
    this.arc(x + d, y + height - d, d, quarter, Math.PI);
    this.lineTo(x, y + a);
    this.arc(x + a, y + a, a, Math.PI, Math.PI + quarter);
    this.closePath();
  }

  closePath() {
    if (!this.current) return;
    this.segments.push('Close');
    this.current = this.start;
  }
}
const pathMethods = ['moveTo', 'lineTo', 'bezierCurveTo', 'quadraticCurveTo', 'arc', 'arcTo', 'ellipse', 'rect', 'roundRect', 'closePath'];

// Path2D keeps its segments in its own coordinates; drawing maps them.
const paths2d = new WeakMap();
class Path2D {
  constructor(path) {
    const builder = new PathBuilder(() => IDENTITY);
    paths2d.set(this, builder);
    if (path instanceof Path2D) builder.segments.push(...paths2d.get(path).segments);
    else if (typeof path === 'string') builder.segments.push(...JSON.parse(page.pathData(path)));
  }

  addPath(path, transform) {
    const matrix = matrixOf(transform);
    const mapped = paths2d.get(path).segments.map((segment) => mapSegment(segment, matrix));
    paths2d.get(this).segments.push(...mapped);
  }
}
for (const name of pathMethods) {
  Path2D.prototype[name] = function (...args) {
    return paths2d.get(this)[name](...args);
  };
}
const mapSegment = (segment, matrix) => {
  if (segment === 'Close') return segment;
  const [name] = Object.keys(segment);
  const values = segment[name];
  const mapped = [];
  for (let index = 0; index < values.length; index += 2) mapped.push(...applyMatrix(matrix, values[index], values[index + 1]));
  return { [name]: mapped };
};

const contexts = new WeakMap();
const fillRule = (rule) => (rule === 'evenodd' ? 'EvenOdd' : 'NonZero');
const keywordSetter = (allowed) => (value) => (allowed.includes(String(value)) ? String(value) : undefined);
const DEFAULT_FONT = { families: ['sans-serif'], size: 10, weight: 400, italic: false };

// The drawing state that save() and restore() keep.
const initialState = () => ({
  matrix: IDENTITY,
  fillStyle: { r: 0, g: 0, b: 0, a: 1 },
  strokeStyle: { r: 0, g: 0, b: 0, a: 1 },
  globalAlpha: 1,
  globalCompositeOperation: 'source-over',
  lineWidth: 1,
  lineCap: 'butt',
  lineJoin: 'miter',
  miterLimit: 10,
  lineDash: [],
  lineDashOffset: 0,
  font: '10px sans-serif',
  parsedFont: DEFAULT_FONT,
  textAlign: 'start',
  textBaseline: 'alphabetic',
  direction: 'inherit',
  imageSmoothingEnabled: true,
  imageSmoothingQuality: 'low',
  shadowBlur: 0,
  shadowColor: 'rgba(0, 0, 0, 0)',
  shadowOffsetX: 0,
  shadowOffsetY: 0,
  filter: 'none',
  letterSpacing: '0px',
  wordSpacing: '0px',
  fontKerning: 'auto',
  fontStretch: 'normal',
  fontVariantCaps: 'normal',
  textRendering: 'auto',
});

class CanvasRenderingContext2D {
  constructor() {
    illegal();
  }

  get canvas() {
    return contexts.get(this).canvas;
  }

  getContextAttributes() {
    return { alpha: true, colorSpace: 'srgb', desynchronized: false, willReadFrequently: false };
  }

  // Drawing state.

  save() {
    const context = contexts.get(this);
    context.stack.push({ ...context.state });
    record(this, 'Save');
  }

  restore() {
    const context = contexts.get(this);
    if (!context.stack.length) return;
    context.state = context.stack.pop();
    record(this, 'Restore');
  }

  reset() {
    const context = contexts.get(this);
    resetContext(context);
    clearBitmap(context);
  }

  isContextLost() {
    return false;
  }

  // Transforms.

  getTransform() {
    return new DOMMatrix(contexts.get(this).state.matrix);
  }

  setTransform(...args) {
    const values = args.length >= 6 ? args.slice(0, 6).map(Number) : matrixOf(args[0]);
    if (finite(...values)) contexts.get(this).state.matrix = values;
  }

  resetTransform() {
    contexts.get(this).state.matrix = IDENTITY;
  }

  transform(a, b, c, d, e, f) {
    const values = [a, b, c, d, e, f].map(Number);
    if (!finite(...values)) return;
    const { state } = contexts.get(this);
    state.matrix = multiplyMatrix(state.matrix, values);
  }

  translate(x, y) {
    this.transform(1, 0, 0, 1, x, y);
  }

  scale(x, y) {
    this.transform(x, 0, 0, y, 0, 0);
  }

  rotate(angle) {
    const [cos, sin] = [Math.cos(angle), Math.sin(angle)];
    this.transform(cos, sin, -sin, cos, 0, 0);
  }

  // Paths.

  beginPath() {
    const context = contexts.get(this);
    context.path = new PathBuilder(() => context.state.matrix);
  }

  fill(...args) {
    const context = contexts.get(this);
    const [path, rule] = args[0] instanceof Path2D ? [segmentsOf(context, args[0]), args[1]] : [context.path.segments, args[0]];
    record(this, {
      Fill: { path, paint: paintOf(context.state.fillStyle, context.state.matrix), rule: fillRule(rule), alpha: context.state.globalAlpha, copy: copying(context) },
    });
  }

  stroke(path) {
    const context = contexts.get(this);
    strokePath(this, path instanceof Path2D ? segmentsOf(context, path) : context.path.segments);
  }

  clip(...args) {
    const context = contexts.get(this);
    const [path, rule] = args[0] instanceof Path2D ? [segmentsOf(context, args[0]), args[1]] : [context.path.segments, args[0]];
    record(this, { Clip: { path, rule: fillRule(rule) } });
  }

  isPointInPath() {
    return false;
  }

  isPointInStroke() {
    return false;
  }

  // Rectangles.

  fillRect(x, y, width, height) {
    const context = contexts.get(this);
    const path = rectangle(context, x, y, width, height);
    if (path) record(this, { Fill: { path, paint: paintOf(context.state.fillStyle, context.state.matrix), rule: 'NonZero', alpha: context.state.globalAlpha, copy: copying(context) } });
  }

  strokeRect(x, y, width, height) {
    const context = contexts.get(this);
    const path = rectangle(context, x, y, width, height);
    if (path) strokePath(this, path);
  }

  clearRect(x, y, width, height) {
    const path = rectangle(contexts.get(this), x, y, width, height);
    if (path) record(this, { Clear: { path } });
  }

  // Text.

  fillText(text, x, y, maxWidth) {
    drawText(this, text, x, y, maxWidth, null);
  }

  strokeText(text, x, y, maxWidth) {
    drawText(this, text, x, y, maxWidth, contexts.get(this).state.lineWidth);
  }

  measureText(text) {
    const { state } = contexts.get(this);
    const [width, ascent, descent] = page.measureText(JSON.stringify(state.parsedFont), String(text));
    const left = { start: 0, left: 0, center: width / 2, end: width, right: width }[state.textAlign];
    return {
      width,
      actualBoundingBoxLeft: left,
      actualBoundingBoxRight: width - left,
      actualBoundingBoxAscent: ascent,
      actualBoundingBoxDescent: descent,
      fontBoundingBoxAscent: ascent,
      fontBoundingBoxDescent: descent,
      emHeightAscent: ascent,
      emHeightDescent: descent,
      hangingBaseline: ascent * 0.8,
      alphabeticBaseline: 0,
      ideographicBaseline: -descent,
    };
  }

  // Images.

  drawImage(image, ...args) {
    const context = contexts.get(this);
    const source = imageSource(image);
    if (!source) return;
    const [width, height] = source.size;
    const [from, to] =
      args.length >= 8
        ? [args.slice(0, 4), args.slice(4, 8)]
        : args.length >= 4
          ? [[0, 0, width, height], args.slice(0, 4)]
          : [[0, 0, width, height], [args[0], args[1], width, height]];
    const rect = ([x, y, w, h]) => ({ x: Number(x), y: Number(y), width: Number(w), height: Number(h) });
    if (!finite(...from, ...to) || !width || !height) return;
    record(this, { Image: { source: source.source, from: rect(from), to: rect(to), transform: context.state.matrix, alpha: context.state.globalAlpha } });
  }

  createImageData(width, height) {
    if (width instanceof ImageData) return new ImageData(width.width, width.height);
    return new ImageData(Math.abs(Math.trunc(width)), Math.abs(Math.trunc(height)));
  }

  getImageData(x, y, width, height) {
    if (!Math.trunc(width) || !Math.trunc(height)) throw new DOMException('The source width or height is 0.', 'IndexSizeError');
    const [left, top] = [Math.min(x, x + width), Math.min(y, y + height)].map(Math.trunc);
    const [w, h] = [Math.abs(Math.trunc(width)), Math.abs(Math.trunc(height))];
    const context = contexts.get(this);
    flush(context, true);
    const bytes = page.canvasRead(context.canvas[ID], left, top, w, h);
    return new ImageData(new Uint8ClampedArray(bytes.buffer), w, h);
  }

  putImageData(data, x, y, dirtyX = 0, dirtyY = 0, dirtyWidth = data.width, dirtyHeight = data.height) {
    const context = contexts.get(this);
    const [left, top] = [Math.max(0, dirtyX), Math.max(0, dirtyY)];
    const right = Math.min(data.width, dirtyX + dirtyWidth);
    const bottom = Math.min(data.height, dirtyY + dirtyHeight);
    if (right <= left || bottom <= top) return;
    const [w, h] = [right - left, bottom - top];
    const pixels = new Uint8Array(w * h * 4);
    for (let row = 0; row < h; row++) {
      const start = ((top + row) * data.width + left) * 4;
      pixels.set(data.data.subarray(start, start + w * 4), row * w * 4);
    }
    flush(context, true);
    page.canvasWrite(context.canvas[ID], Math.trunc(x) + left, Math.trunc(y) + top, w, h, pixels);
    context.drawn = true;
  }

  // Styles.

  createLinearGradient(x0, y0, x1, y1) {
    if (!finite(x0, y0, x1, y1)) throw new TypeError('The provided value is non-finite.');
    return createGradient('linear', [x0, y0, x1, y1].map(Number));
  }

  createRadialGradient(x0, y0, r0, x1, y1, r1) {
    if (!finite(x0, y0, r0, x1, y1, r1)) throw new TypeError('The provided value is non-finite.');
    if (r0 < 0 || r1 < 0) throw new DOMException('The radius provided is negative.', 'IndexSizeError');
    return createGradient('radial', [x0, y0, r0, x1, y1, r1].map(Number));
  }

  createConicGradient(angle, x, y) {
    return createGradient('linear', [x, y, x + Math.cos(angle), y + Math.sin(angle)].map(Number));
  }

  createPattern() {
    return Object.create(CanvasPattern.prototype);
  }

  setLineDash(segments) {
    const values = [...segments].map(Number);
    if (!values.every((value) => Number.isFinite(value) && value >= 0)) return;
    contexts.get(this).state.lineDash = values.length % 2 ? [...values, ...values] : values;
  }

  getLineDash() {
    return [...contexts.get(this).state.lineDash];
  }
}
for (const name of pathMethods) {
  if (name === 'closePath') continue;
  CanvasRenderingContext2D.prototype[name] = function (...args) {
    return contexts.get(this).path[name](...args);
  };
}
CanvasRenderingContext2D.prototype.closePath = function () {
  contexts.get(this).path.closePath();
};

// Style properties: getters serialize, setters validate and ignore invalid
// values like browsers do.
const styleProperty = (name) => ({
  get() {
    const value = contexts.get(this).state[name];
    return value && 'r' in value ? serializeColor(value) : value;
  },
  set(value) {
    if (gradients.has(value) || value instanceof CanvasPattern) {
      contexts.get(this).state[name] = value;
      return;
    }
    const parsed = parseCanvasColor(value);
    if (parsed) contexts.get(this).state[name] = parsed;
  },
});
const stateProperty = (name, validate) => ({
  get() {
    return contexts.get(this).state[name];
  },
  set(value) {
    const valid = validate(value);
    if (valid !== undefined) contexts.get(this).state[name] = valid;
  },
});
const positive = (value) => (Number.isFinite(Number(value)) && Number(value) > 0 ? Number(value) : undefined);
const anyNumber = (value) => (Number.isFinite(Number(value)) ? Number(value) : undefined);
Object.defineProperties(CanvasRenderingContext2D.prototype, {
  fillStyle: styleProperty('fillStyle'),
  strokeStyle: styleProperty('strokeStyle'),
  globalAlpha: stateProperty('globalAlpha', (value) => (Number(value) >= 0 && Number(value) <= 1 ? Number(value) : undefined)),
  globalCompositeOperation: stateProperty(
    'globalCompositeOperation',
    keywordSetter(['source-over', 'source-in', 'source-out', 'source-atop', 'destination-over', 'destination-in', 'destination-out', 'destination-atop', 'lighter', 'copy', 'xor', 'multiply', 'screen', 'overlay', 'darken', 'lighten']),
  ),
  lineWidth: stateProperty('lineWidth', positive),
  lineCap: stateProperty('lineCap', keywordSetter(['butt', 'round', 'square'])),
  lineJoin: stateProperty('lineJoin', keywordSetter(['miter', 'round', 'bevel'])),
  miterLimit: stateProperty('miterLimit', positive),
  lineDashOffset: stateProperty('lineDashOffset', anyNumber),
  textAlign: stateProperty('textAlign', keywordSetter(['start', 'end', 'left', 'right', 'center'])),
  textBaseline: stateProperty('textBaseline', keywordSetter(['top', 'hanging', 'middle', 'alphabetic', 'ideographic', 'bottom'])),
  direction: stateProperty('direction', keywordSetter(['ltr', 'rtl', 'inherit'])),
  imageSmoothingEnabled: stateProperty('imageSmoothingEnabled', Boolean),
  imageSmoothingQuality: stateProperty('imageSmoothingQuality', keywordSetter(['low', 'medium', 'high'])),
  shadowBlur: stateProperty('shadowBlur', anyNumber),
  shadowColor: stateProperty('shadowColor', String),
  shadowOffsetX: stateProperty('shadowOffsetX', anyNumber),
  shadowOffsetY: stateProperty('shadowOffsetY', anyNumber),
  filter: stateProperty('filter', String),
  letterSpacing: stateProperty('letterSpacing', String),
  wordSpacing: stateProperty('wordSpacing', String),
  fontKerning: stateProperty('fontKerning', String),
  fontStretch: stateProperty('fontStretch', String),
  fontVariantCaps: stateProperty('fontVariantCaps', String),
  textRendering: stateProperty('textRendering', String),
  font: {
    get() {
      return contexts.get(this).state.font;
    },
    set(value) {
      const text = String(value).trim();
      const parsed = page.parseFont(text);
      if (parsed === null) return;
      Object.assign(contexts.get(this).state, { font: text, parsedFont: JSON.parse(parsed) });
    },
  },
});

const copying = (context) => context.state.globalCompositeOperation === 'copy';

// The segments of a Path2D under the current transform.
const segmentsOf = (context, path) => paths2d.get(path).segments.map((segment) => mapSegment(segment, context.state.matrix));

// A transformed rectangle path, or null for non-finite values.
const rectangle = (context, x, y, width, height) => {
  if (!finite(x, y, width, height)) return null;
  const builder = new PathBuilder(() => context.state.matrix);
  builder.rect(Number(x), Number(y), Number(width), Number(height));
  return builder.segments;
};

const strokePath = (target, path) => {
  const { state } = contexts.get(target);
  record(target, {
    Stroke: {
      path,
      paint: paintOf(state.strokeStyle, IDENTITY),
      width: state.lineWidth,
      cap: { butt: 'Butt', round: 'Round', square: 'Square' }[state.lineCap],
      join: { miter: 'Miter', round: 'Round', bevel: 'Bevel' }[state.lineJoin],
      miter: state.miterLimit,
      dash: state.lineDash,
      offset: state.lineDashOffset,
      transform: state.matrix,
      alpha: state.globalAlpha,
      copy: state.globalCompositeOperation === 'copy',
    },
  });
};

const drawText = (target, text, x, y, maxWidth, stroke) => {
  const { state } = contexts.get(target);
  if (!finite(x, y)) return;
  const content = String(text).replace(/[\t\n\f\r]/g, ' ');
  const [width, ascent, descent] = page.measureText(JSON.stringify(state.parsedFont), content);
  const shift = { top: ascent, hanging: ascent * 0.8, middle: (ascent - descent) / 2, alphabetic: 0, ideographic: -descent, bottom: -descent }[state.textBaseline];
  const rtl = state.direction === 'rtl';
  const anchor = { start: rtl ? 'Right' : 'Left', end: rtl ? 'Left' : 'Right', left: 'Left', right: 'Right', center: 'Center' }[state.textAlign];
  let matrix = state.matrix;
  if (maxWidth !== undefined && Number(maxWidth) < width) {
    const squeeze = Math.max(0, Number(maxWidth)) / width;
    matrix = multiplyMatrix(matrix, [squeeze, 0, 0, 1, Number(x) * (1 - squeeze), 0]);
  }
  record(target, {
    Text: {
      text: content,
      x: Number(x),
      y: Number(y) + shift,
      anchor,
      font: state.parsedFont,
      paint: paintOf(stroke === null ? state.fillStyle : state.strokeStyle, IDENTITY),
      stroke,
      transform: matrix,
      alpha: state.globalAlpha,
    },
  });
};

// What drawImage reads: an image URL or another canvas.
const imageSource = (image) => {
  if (image instanceof HTMLImageElement) {
    const url = resolveURL(image.currentSrc || image.getAttribute('src') || '');
    const size = page.imageSize(url);
    return size && { source: { Url: url }, size };
  }
  if (image instanceof HTMLCanvasElement) {
    const context = contexts.get(contextOf(image));
    if (context) flush(context, true);
    return { source: { Canvas: image[ID] }, size: [image.width, image.height] };
  }
  if (image instanceof ImageData || image === null || image === undefined) {
    throw new TypeError("Failed to execute 'drawImage': The provided value is not of a supported image type.");
  }
  return null;
};

// Commands wait in the context until the task ends or the bitmap is read.
const record = (target, command) => {
  const context = contexts.get(target);
  context.commands.push(command);
  if (!context.scheduled) {
    context.scheduled = true;
    queueMicrotask(() => flush(context, false));
  }
};

const flush = (context, force) => {
  context.scheduled = false;
  if (!context.commands.length && !(force && !context.drawn)) return;
  const commands = JSON.stringify(context.commands);
  context.commands = [];
  context.drawn = true;
  page.canvasDraw(context.canvas[ID], context.canvas.width, context.canvas.height, commands);
};

const resetContext = (context) => {
  context.state = initialState();
  context.stack = [];
  context.commands = [];
  context.path = new PathBuilder(() => context.state.matrix);
};

// Starts the bitmap over: a zero size drops it, the real size recreates it.
const clearBitmap = (context) => {
  page.canvasDraw(context.canvas[ID], 0, 0, '[]');
  context.drawn = false;
  flush(context, true);
};

// The 2D context of a canvas element, created on first use.
const canvasContexts = new WeakMap();
const contextOf = (canvas) => canvasContexts.get(canvas);
const resizeCanvas = (canvas) => {
  const target = canvasContexts.get(canvas);
  if (!target) return;
  const context = contexts.get(target);
  resetContext(context);
  clearBitmap(context);
};
const getContext2d = (canvas) => {
  let target = canvasContexts.get(canvas);
  if (!target) {
    target = Object.create(CanvasRenderingContext2D.prototype);
    const context = { canvas, drawn: false, scheduled: false };
    resetContext(context);
    contexts.set(target, context);
    canvasContexts.set(canvas, target);
  }
  return target;
};

// A canvas's pixels as a data URL.
const canvasDataURL = (canvas, type) => {
  if (!canvas.width || !canvas.height) return 'data:,';
  const target = canvasContexts.get(canvas);
  if (target) flush(contexts.get(target), true);
  return page.canvasURL(canvas[ID], type === 'image/jpeg');
};

class ImageData {
  constructor(data, width, height) {
    if (data instanceof Uint8ClampedArray) {
      const w = Math.trunc(width);
      const h = height === undefined ? data.length / 4 / w : Math.trunc(height);
      if (!w || !Number.isInteger(h) || data.length !== w * h * 4) throw new DOMException('The input data length is not equal to (4 * width * height).', 'IndexSizeError');
      Object.assign(this, { data, width: w, height: h });
    } else {
      const [w, h] = [Math.trunc(data), Math.trunc(width)];
      if (!(w > 0 && h > 0)) throw new DOMException('The source width or height is 0.', 'IndexSizeError');
      Object.assign(this, { data: new Uint8ClampedArray(w * h * 4), width: w, height: h });
    }
    this.colorSpace = 'srgb';
  }
}
