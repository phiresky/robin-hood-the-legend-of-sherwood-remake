import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';

export function cspDirectives(html: string): Map<string, Set<string>> {
    const document = JSDOM.fragment(html);
    const content = document.querySelector('meta[http-equiv="Content-Security-Policy" i]')?.getAttribute('content');
    assert.ok(content, 'document must declare a Content-Security-Policy');
    const directives = new Map<string, Set<string>>();
    for (const directive of content.split(';')) {
        const [name, ...sources] = directive.trim().split(/\s+/u);
        if (name && !directives.has(name.toLowerCase())) {
            directives.set(name.toLowerCase(), new Set(sources));
        }
    }
    return directives;
}
