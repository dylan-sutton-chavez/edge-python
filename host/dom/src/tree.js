/* Selection, traversal, mutation, content, attrs, classes, dataset. */

export default ({ alloc, node, allocList, cleanSubtree }) => ({
    query: (sel) => alloc(document.querySelector(sel)),
    query_all: (sel) => allocList(document.querySelectorAll(sel)),
    closest: (h, sel) => alloc(node(h).closest(sel)),
    matches: (h, sel) => node(h).matches(sel),
    body: () => alloc(document.body),
    active_element: () => alloc(document.activeElement),

    parent: (h) => alloc(node(h).parentElement),
    children: (h) => allocList(node(h).children),
    first_child: (h) => alloc(node(h).firstElementChild),
    last_child: (h) => alloc(node(h).lastElementChild),
    next_sibling: (h) => alloc(node(h).nextElementSibling),
    prev_sibling: (h) => alloc(node(h).previousElementSibling),
    tag_name: (h) => (node(h).tagName || '').toLowerCase(),

    create_element: (tag) => alloc(document.createElement(tag)),
    /* SVG/MathML need a namespace; `createElement("svg")` returns an HTMLUnknownElement that won't render. */
    create_element_ns: (ns, tag) => alloc(document.createElementNS(ns, tag)),

    append_child: (parent, child) => { node(parent).appendChild(node(child)); },
    /* Sibling positioning without needing the parent handle. */
    insert_before: (newH, refH) => {
        const refNode = node(refH);
        refNode.parentNode.insertBefore(node(newH), refNode);
    },
    // Sweeps subtree handles, bindings, animations.
    remove: (h) => {
        const el = node(h);
        el.remove();
        cleanSubtree(el);
    },
    // Variadic: first arg is parent, rest are child handles. Pass only parent to clear. Sweeps detached children.
    replace_children: (parent, ...kids) => {
        const p = node(parent);
        const old = Array.from(p.children);
        p.replaceChildren(...kids.map(node));
        for (const c of old) cleanSubtree(c);
    },
    /* `deep` defaults to true (descendants included). */
    clone_node: (h, deep) => alloc(node(h).cloneNode(deep === undefined ? true : deep)),

    get_text: (h) => node(h).textContent || '',
    set_text: (h, txt) => {
        const el = node(h);
        const old = el.children.length ? Array.from(el.children) : null;
        el.textContent = txt;
        if (old) for (const c of old) cleanSubtree(c);
    },

    get_html: (h) => node(h).innerHTML || '',
    set_html: (h, html) => {
        const el = node(h);
        const old = el.children.length ? Array.from(el.children) : null;
        el.innerHTML = html;
        if (old) for (const c of old) cleanSubtree(c);
    },

    get_attribute: (h, name) => {
        const v = node(h).getAttribute(name);
        return v === null ? null : v;
    },
    set_attribute: (h, name, value) => { node(h).setAttribute(name, value); },
    remove_attribute: (h, name) => { node(h).removeAttribute(name); },

    add_class: (h, c) => { node(h).classList.add(c); },
    remove_class: (h, c) => { node(h).classList.remove(c); },
    toggle_class: (h, c) => node(h).classList.toggle(c),
    has_class: (h, c) => node(h).classList.contains(c),

    get_data: (h, key) => {
        const v = node(h).dataset[key];
        return v === undefined ? null : v;
    },
    set_data: (h, key, value) => { node(h).dataset[key] = value; },
});
