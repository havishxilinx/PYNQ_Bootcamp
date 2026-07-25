"""Pre-game riddle + free-hint LLM solvers -- console port of notebook
Sections 7a and 7b. Merged into one module since they share _llm_call_lock
and the bootcamp_ai import/threading pattern.

Requires bootcamp_ai importable -- main.py adds
/home/root/jupyter_notebooks/PYNQ_Bootcamp/bootcamp_sessions/ai_llm to
sys.path before this module is imported, same as the notebook's Section 1
setup cell did.
"""
import json
import re
import queue
import threading

import bootcamp_ai

# -- Section 7a: pre-game riddle -------------------------------------------

PREGAME_RIDDLE_MODEL = 'Smart Helper'  # bootcamp_ai friendly model name
PREGAME_RIDDLE_TIMEOUT_SECONDS = 20

# Serializes every bootcamp_ai call across both LLM-backed solvers below --
# use_model()/MAX_TOKENS are global halo Strix settings, and the pre-game
# riddle and free hint can both arrive in the same pre-game window on
# separate background threads.
_llm_call_lock = threading.Lock()


def build_pregame_riddle_prompt(riddle_text):
    return (
        'You are a fast riddle-solving assistant for a live competition. '
        'A player was just given this riddle and needs the answer immediately '
        'so they can say it out loud before their opponent:\n\n'
        f'"{riddle_text}"\n\n'
        'Reply with ONLY compact JSON, no extra words, in exactly this shape:\n'
        '{"answer": "<short answer, one or two words>"}'
    )


def _extract_json_object(reply_text):
    """Extract the first valid JSON object, including from fenced/prose replies."""
    text = str(reply_text or '')
    decoder = json.JSONDecoder()
    for match in re.finditer(r'\{', text):
        try:
            value, _ = decoder.raw_decode(text[match.start():])
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            return value
    raise ValueError(f'LLM reply did not contain valid JSON: {reply_text!r}')


def _fallback_answer_from_llm(reply_text):
    """Recover a short answer from common non-JSON LLM responses."""
    text = str(reply_text or '').strip()
    text = re.sub(r'^```(?:json)?\s*|\s*```$', '', text, flags=re.IGNORECASE).strip()
    match = re.search(r'["\']?answer["\']?\s*[:=]\s*["\']?([^"\'\n}\]]+)', text, re.IGNORECASE)
    if not match:
        match = re.search(r'\banswer\s+is\s+([^\n.!?]+)', text, re.IGNORECASE)
    if match:
        answer = match.group(1)
    else:
        lines = [line.strip() for line in text.splitlines() if line.strip()]
        answer = lines[0] if lines else ''
    answer = re.sub(r'^(?:answer\s*[:=-]\s*)', '', answer, flags=re.IGNORECASE)
    answer = answer.strip().strip('`"\'{}[]').rstrip('.,;:').strip()
    if not answer:
        raise ValueError(f'LLM reply did not contain a usable answer: {reply_text!r}')
    return answer[:80].strip()


def _prompt_llm_with_timeout(prompt_text, timeout_seconds):
    """Run the helper in a daemon thread so its 300s HTTP timeout cannot hang the console app."""
    result_queue = queue.Queue(maxsize=1)

    def worker():
        try:
            reply = bootcamp_ai.prompt(
                prompt_text,
                enforce_cooldown=False,
                session='pregame_riddle',
            )
            result_queue.put(('ok', reply))
        except Exception as exc:
            result_queue.put(('error', exc))

    threading.Thread(target=worker, daemon=True, name='pregame-riddle-llm').start()
    try:
        status, payload = result_queue.get(timeout=float(timeout_seconds))
    except queue.Empty as exc:
        raise TimeoutError(f'LLM did not respond within {timeout_seconds} seconds.') from exc
    if status == 'error':
        raise payload
    return payload


def solve_pregame_riddle_with_llm(riddle_text, model=None):
    """Return a best-guess answer without allowing a stalled or malformed LLM
    response to hang the match client."""
    riddle_text = str(riddle_text).strip()
    if not riddle_text:
        raise ValueError('Riddle text is empty.')
    with _llm_call_lock:
        bootcamp_ai.use_model(model or PREGAME_RIDDLE_MODEL)
        reply_text = _prompt_llm_with_timeout(
            build_pregame_riddle_prompt(riddle_text),
            PREGAME_RIDDLE_TIMEOUT_SECONDS,
        )
    used_fallback = False
    try:
        parsed = _extract_json_object(reply_text)
        answer = str(parsed.get('answer') or '').strip()
        if not answer:
            raise ValueError('JSON answer was empty.')
    except (TypeError, ValueError, json.JSONDecodeError):
        answer = _fallback_answer_from_llm(reply_text)
        used_fallback = True
    return {
        'riddle': riddle_text,
        'answer': answer,
        'raw_reply': reply_text,
        'used_fallback': used_fallback,
    }


# -- Section 7b: free hint --------------------------------------------------

FREE_HINT_ITEM_MODEL = 'Quick Helper'
FREE_HINT_DIRECTION_MODEL = 'Quick Helper'  # the 27B was slow (15s) AND wrong here
FREE_HINT_MAX_TOKENS = 60
FREE_HINT_TIMEOUT_SECONDS = 20
FREE_HINT_QUADRANTS = ['top left', 'top right', 'bottom left', 'bottom right']

# A free hint is one string carrying an OBJECT and a grid LOCATION, e.g.
# "I am a horse. I sit where the sun rises and the ceiling is closest --
# the first page of a book, the corner where reading begins." (= top left).
# The location riddles use a consistent reading-order convention:
#   begins / first / where reading starts -> LEFT
#   ends / last / last word                -> RIGHT
#   ceiling / high / sky                    -> TOP
#   floor / low / ground                    -> BOTTOM
# We deliberately have the LLM reason this out (no lookup table) rather than
# hardcode the referee's current riddle wording, which could change.
_FREE_HINT_DIRECTION_FRAME = (
    "Think of the grid like a page of text you read left-to-right, top-to-bottom. "
    "Decide the vertical half and the horizontal half separately, then combine.\n"
    "VERTICAL: 'ceiling', 'high', 'sky', 'sun rises', 'first line' mean TOP; "
    "'floor', 'low', 'ground', 'last line' mean BOTTOM.\n"
    "HORIZONTAL: 'begins', 'starts', 'first page', 'where reading begins' mean LEFT; "
    "'ends', 'last word', 'last page', 'where the line ends' mean RIGHT.\n"
    "IMPORTANT: 'where the last line BEGINS' = BOTTOM + LEFT (last=bottom, begins=left). "
    "'where the first line ENDS' = TOP + RIGHT (first=top, ends=right). "
    "Ignore relative phrases like 'across from me' or 'above me' -- rely only on the "
    "reading-order clues above."
)


def _run_with_timeout(target, timeout_seconds):
    """Runs target() in a daemon thread and enforces a timeout, so a stalled
    LLM call cannot hang the match client (generalizes the pre-game riddle
    solver's _prompt_llm_with_timeout to any callable)."""
    result_queue = queue.Queue(maxsize=1)

    def worker():
        try:
            result_queue.put(('ok', target()))
        except Exception as exc:
            result_queue.put(('error', exc))

    threading.Thread(target=worker, daemon=True, name='free-hint-llm').start()
    try:
        status, payload = result_queue.get(timeout=float(timeout_seconds))
    except queue.Empty as exc:
        raise TimeoutError(f'LLM did not respond within {timeout_seconds} seconds.') from exc
    if status == 'error':
        raise payload
    return payload


def _solve_free_hint_piece(hint, instruction, model):
    """Shared engine: pick model, cap tokens, one locked+timeout-guarded call."""
    def call():
        with _llm_call_lock:
            bootcamp_ai.use_model(model)
            old_max_tokens = bootcamp_ai.MAX_TOKENS
            bootcamp_ai.MAX_TOKENS = FREE_HINT_MAX_TOKENS
            try:
                return bootcamp_ai.prompt(
                    f'{instruction}\n\nRiddle: "{hint}" /no_think',
                    enforce_cooldown=False, session='free_hint')
            finally:
                bootcamp_ai.MAX_TOKENS = old_max_tokens

    reply = _run_with_timeout(call, FREE_HINT_TIMEOUT_SECONDS)
    return (reply or '').strip()


def _last_quadrant(text):
    """Return the LAST quadrant mentioned (the model's conclusion), or None.
    Scanning for the last occurrence avoids matching a quadrant that appears
    earlier inside reasoning like 'not top left, but bottom right'."""
    best_q, best_pos = None, -1
    for q in FREE_HINT_QUADRANTS:
        pos = text.rfind(q)
        if pos > best_pos:
            best_q, best_pos = q, pos
    return best_q


def solve_free_hint_item(hint):
    """Extract just the OBJECT from a full free-hint riddle -> one lowercase word."""
    reply = _solve_free_hint_piece(
        hint,
        "This riddle describes an OBJECT and a LOCATION. Ignore the location. "
        "What single object is being described? Reply with ONLY one lowercase "
        "word, no emojis, no punctuation.",
        FREE_HINT_ITEM_MODEL)
    first = reply.split()[0] if reply else ''
    return re.sub(r'[^a-zA-Z]', '', first).lower()


def solve_free_hint_direction(hint):
    """Extract just the QUADRANT from a full free-hint riddle -> one of FREE_HINT_QUADRANTS."""
    reply = _solve_free_hint_piece(
        hint,
        "This riddle describes an OBJECT and a LOCATION on a grid. Ignore the object. "
        f"{_FREE_HINT_DIRECTION_FRAME} "
        "The location is always one of the four CORNERS. Using the clues, work out "
        "the corner and reply with ONLY one of exactly these choices: "
        "top left, top right, bottom left, bottom right. No other words.",
        FREE_HINT_DIRECTION_MODEL)
    cleaned = re.sub(r'[^a-z ]', ' ', reply.lower())
    cleaned = re.sub(r'\s+', ' ', cleaned).strip()
    quadrant = _last_quadrant(cleaned)
    if quadrant:
        return quadrant
    vertical = 'top' if ('top' in cleaned or 'up' in cleaned) else \
        ('bottom' if ('bottom' in cleaned or 'down' in cleaned) else '')
    horizontal = 'left' if 'left' in cleaned else ('right' if 'right' in cleaned else '')
    if vertical and horizontal:
        return f'{vertical} {horizontal}'
    return None


def solve_free_hint(hint):
    """Full free-hint text -> {'item': ..., 'direction': ...}."""
    return {'item': solve_free_hint_item(hint), 'direction': solve_free_hint_direction(hint)}
