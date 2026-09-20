const CORS = { "Access-Control-Allow-Origin": "*" };
const encoder = new TextEncoder();

function text(body: string, type: string): Response {
    return new Response(body, { headers: { "Content-Type": type, ...CORS } });
}

// Three data events, then the stream stays open until the client goes away.
function sse(): Response {
    const stream = new ReadableStream<Uint8Array>({
        start(controller) {
            for (let i = 1; i <= 3; i++) controller.enqueue(encoder.encode(`id: ${i}\ndata: event ${i}\n\n`));
        },
    });
    return new Response(stream, { headers: { "Content-Type": "text/event-stream", "Cache-Control": "no-cache", ...CORS } });
}

// Parts arrive as separate reads, one splits a CRLF, then the stream stays open.
const FIELDS = [
    "\uFEFFdata: bom first\n\n",
    ": a comment\r",
    "\nevent: ping\r\ndata: skipped\r\n\r\n",
    "id: 7\rdata: first line\rdata: second line\r\r",
    "data:no space\nretry: 5000\n\n",
    "id\ndata: after empty id\n\n",
];

function fields(): Response {
    const stream = new ReadableStream<Uint8Array>({
        async start(controller) {
            try {
                for (const part of FIELDS) {
                    controller.enqueue(encoder.encode(part));
                    await new Promise((resolve) => setTimeout(resolve, 10));
                }
            } catch {
                // The client went away before the last part.
            }
        },
    });
    return new Response(stream, { headers: { "Content-Type": "text/event-stream", ...CORS } });
}

// The first connection ends after one event, the retry echoes the Last-Event-ID it carried.
function reconnect(req: Request): Response {
    const last = req.headers.get("last-event-id");
    const headers = { "Content-Type": "text/event-stream", ...CORS };
    if (last === null) return new Response("retry: 250\nid: 41\ndata: first\n\n", { headers });
    const stream = new ReadableStream<Uint8Array>({
        start(controller) { controller.enqueue(encoder.encode(`data: last=${last}\n\n`)); },
    });
    return new Response(stream, { headers });
}

// Echoes text frames, the fixture answers close and ping by itself.
function ws(req: Request): Response {
    const { socket, response } = Deno.upgradeWebSocket(req);
    socket.onmessage = (e) => { if (typeof e.data === "string") socket.send(e.data); };
    return response;
}

// Canned http and sse plus a ws echo over loopback for the system suite.
function handle(req: Request): Response {
    if (req.method === "OPTIONS") {
        return new Response(null, { status: 204, headers: { "Access-Control-Allow-Methods": "GET", "Access-Control-Allow-Headers": "*", ...CORS } });
    }
    switch (new URL(req.url).pathname) {
        case "/text": return text("hello from mock", "text/plain");
        case "/json": return text('{"ok":true}', "application/json");
        case "/sse": return sse();
        case "/sse-fields": return fields();
        case "/sse-reconnect": return reconnect(req);
        case "/ws": return ws(req);
        default: return new Response(null, { status: 404 });
    }
}

// The harness reads the port from the first stdout line.
Deno.serve({ hostname: "127.0.0.1", port: 0, onListen: ({ port }) => console.log(port) }, handle);
