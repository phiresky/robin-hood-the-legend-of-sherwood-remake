export type ElementOptions = {
    readonly className?: string;
    readonly text?: string;
    readonly attrs?: Readonly<Record<string, string>>;
};

export function statePanel(title: string, description: string): HTMLElement {
    return element('section', { className: 'panel state-panel' }, [element('h2', { text: title }), element('p', { text: description })]);
}

/** Build DOM without interpreting API-controlled strings as markup. */
export function element<K extends keyof HTMLElementTagNameMap>(
    tag: K,
    options: ElementOptions = {},
    children: readonly (Node | string)[] = [],
): HTMLElementTagNameMap[K] {
    const node = document.createElement(tag);
    if (options.className !== undefined) {
        node.className = options.className;
    }
    if (options.text !== undefined) {
        node.textContent = options.text;
    }
    for (const [name, value] of Object.entries(options.attrs ?? {})) {
        node.setAttribute(name, value);
    }
    for (const child of children) {
        node.append(child);
    }
    return node;
}

export function replace(target: Element, ...children: readonly Node[]): void {
    target.replaceChildren(...children);
}

export function link(label: string, href: string, className?: string): HTMLAnchorElement {
    const anchor = element('a', { text: label, attrs: { href } });
    if (className !== undefined) {
        anchor.className = className;
    }
    return anchor;
}
