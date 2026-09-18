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

// Echoes text frames, the fixture answers close and ping by itself.
function ws(req: Request): Response {
    const { socket, response } = Deno.upgradeWebSocket(req);
    socket.onmessage = (e) => { if (typeof e.data === "string") socket.send(e.data); };
    return response;
}

// Canned http and sse plus a ws echo over loopback for the system suite.
function handle(req: Request): Response {
    if (req.method === "OPTIONS") {
        return new Response(null, { status: 204, headers: { "Access-Control-Allow-Methods": "GET", ...CORS } });
    }
    switch (new URL(req.url).pathname) {
        case "/text": return text("hello from mock", "text/plain");
        case "/json": return text('{"ok":true}', "application/json");
        case "/sse": return sse();
        case "/ws": return ws(req);
        default: return new Response(null, { status: 404 });
    }
}

// The harness reads the port from the first stdout line.
Deno.serve({ hostname: "127.0.0.1", port: 0, onListen: ({ port }) => console.log(port) }, handle);
