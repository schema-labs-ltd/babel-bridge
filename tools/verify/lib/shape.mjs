import { readFileSync } from "node:fs";

// Compares live JSON with the shapes pinned in fixtures/api/responses, which
// the Apple apps decode (APIModels.swift). Every key the fixture sets to a
// non-null value must be present in the live value with the same JSON type.
// A live null is accepted where the fixture holds a scalar, because the
// fixtures cannot say which fields are optional, but not where it holds an
// object or array, which the app models decode as required. Extra live keys are accepted, as the decoders ignore
// them. Array items must match at least one item of the fixture array.
// Objects under an OPEN key (a message's `content`, whose fields depend on the
// message type and are all optional in the app) may leave keys out.

export const OPEN_KEYS = new Set(["content"]);

const kind = (value) =>
  value === null ? "null" : Array.isArray(value) ? "array" : typeof value;

export function shapeProblems(fixture, live, path = "$", open = false) {
  if (fixture === null) return [];
  const want = kind(fixture);
  if (live === null) {
    return want === "object" || want === "array"
      ? [`${path}: expected ${want}, got null`]
      : [];
  }
  const got = kind(live);
  if (want !== got) return [`${path}: expected ${want}, got ${got}`];
  if (want === "array") {
    if (fixture.length === 0) return [];
    const problems = [];
    live.forEach((item, index) => {
      const options = fixture.map((sample) =>
        shapeProblems(sample, item, `${path}[${index}]`),
      );
      if (options.every((option) => option.length > 0)) {
        // Report the closest fixture item, the one with fewest problems.
        options.sort((a, b) => a.length - b.length);
        problems.push(...options[0]);
      }
    });
    return problems;
  }
  if (want === "object") {
    const problems = [];
    for (const [key, value] of Object.entries(fixture)) {
      if (value === null) continue;
      if (!(key in live)) {
        if (open) continue;
        problems.push(`${path}.${key}: missing`);
        continue;
      }
      problems.push(
        ...shapeProblems(
          value,
          live[key],
          `${path}.${key}`,
          OPEN_KEYS.has(key),
        ),
      );
    }
    return problems;
  }
  return [];
}

export function loadFixture(root, name) {
  return JSON.parse(
    readFileSync(new URL(`fixtures/api/responses/${name}.json`, root), "utf8"),
  );
}
