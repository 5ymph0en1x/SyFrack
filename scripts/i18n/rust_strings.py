"""Small Rust string lexer for the localization audit (no third-party packages)."""
import re
CHAR = re.compile(r"'(?:\\(?:u\{[0-9a-fA-F]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'")
RAW = re.compile(r'(?:br|r)(#{0,255})"')


def strings(source):
    """Yield (start, end, decoded text), excluding comments and character literals."""
    i = 0
    while i < len(source):
        if source.startswith('//', i):
            end = source.find('\n', i)
            i = len(source) if end < 0 else end + 1
            continue
        if source.startswith('/*', i):
            depth = 1
            i += 2
            while i < len(source) and depth:
                if source.startswith('/*', i):
                    depth += 1
                    i += 2
                elif source.startswith('*/', i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            continue
        if source[i] == "'":
            char = CHAR.match(source, i)
            if char:
                i = char.end()
                continue
        raw = RAW.match(source, i)
        if raw:
            end = source.find('"' + raw[1], raw.end())
            if end < 0:
                raise ValueError('Unclosed raw Rust string')
            stop = end + 1 + len(raw[1])
            yield i, stop, source[raw.end():end]
            i = stop
            continue
        if source[i] == '"':
            start = i
            i += 1
            text = ''
            while i < len(source) and source[i] != '"':
                if source[i] != '\\':
                    text += source[i]
                    i += 1
                    continue
                i += 1
                c = source[i]
                if c in '\n\r':
                    while i < len(source) and source[i].isspace():
                        i += 1
                    continue
                if c == 'u':
                    end = source.index('}', i)
                    text += chr(int(source[i + 2:end].replace('_', ''), 16))
                    i = end + 1
                elif c == 'x':
                    text += chr(int(source[i + 1:i + 3], 16))
                    i += 3
                else:
                    text += {'n': '\n', 'r': '\r', 't': '\t', '0': '\0'}.get(c, c)
                    i += 1
            yield start, i + 1, text
            i += 1
            continue
        i += 1


def production_source(source):
    """Mask test modules without discarding production code following them."""
    mask = list(source)
    def blank(start, end):
        for i in range(start, end):
            if mask[i] != '\n':
                mask[i] = ' '
    for start, end, _ in strings(source):
        blank(start, end)
    # At this point string contents cannot be confused with comment markers.
    text = ''.join(mask)
    i = 0
    while i < len(text):
        if text.startswith('//', i):
            end = text.find('\n', i)
            end = len(text) if end < 0 else end
            blank(i, end)
            i = end
        elif text.startswith('/*', i):
            start, depth = i, 1
            i += 2
            while i < len(text) and depth:
                if text.startswith('/*', i): depth += 1; i += 2
                elif text.startswith('*/', i): depth -= 1; i += 2
                else: i += 1
            blank(start, i)
        else:
            char = CHAR.match(text, i)
            if char:
                blank(i, char.end())
                i = char.end()
            else:
                i += 1
    code = ''.join(mask)
    result = list(source)
    for match in re.finditer(r'#\[cfg\(test\)\]\s*mod\s+\w+\s*\{', code):
        depth, end = 1, match.end()
        while depth:
            if code[end] == '{': depth += 1
            elif code[end] == '}': depth -= 1
            end += 1
        for i in range(match.start(), end):
            if result[i] != '\n': result[i] = ' '
    return ''.join(result)
