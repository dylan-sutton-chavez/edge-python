export const ui = () => ({
    render: (text) => { document.querySelector("#app").textContent = text; },
    upper: (s) => s.toUpperCase(),
    echo: (v) => v,
    jstype: (v) => typeof v,
});
