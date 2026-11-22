// Turns a public request into the request the container's server receives.

// Headers a client could send to change how the server sees the request. The
// Worker sets the forwarded ones itself from the real request.
const CLIENT_CONTROLLED = [
  "forwarded",
  "x-forwarded-host",
  "x-forwarded-proto",
  "x-forwarded-for",
];

export function forwardedRequest(request: Request): Request {
  const publicUrl = new URL(request.url);
  const url = new URL(
    publicUrl.pathname + publicUrl.search,
    "http://container",
  );
  const forwarded = new Request(url, request);
  for (const name of CLIENT_CONTROLLED) forwarded.headers.delete(name);
  forwarded.headers.delete("host");
  // The server builds OAuth and MCP URLs from the host it is called on, which
  // axum reads from X-Forwarded-Host before Host.
  forwarded.headers.set("x-forwarded-host", publicUrl.host);
  forwarded.headers.set(
    "x-forwarded-proto",
    publicUrl.protocol.replace(":", ""),
  );
  const clientIp = request.headers.get("cf-connecting-ip");
  if (clientIp) forwarded.headers.set("x-forwarded-for", clientIp);
  return forwarded;
}
