def classify(items, mode, strict, verbose, retries, fallback):
    result = []
    for item in items:
        if item is None:
            continue
        if mode == "a":
            if strict and item > 10:
                result.append("big")
            elif item > 5:
                result.append("mid")
            else:
                result.append("small")
        elif mode == "b":
            for _ in range(retries):
                if verbose or fallback:
                    result.append("retry")
        else:
            while item > 0:
                item -= 1
                if item % 2 == 0 and item % 3 == 0:
                    result.append("six")
    return result


def tidy(x: int) -> int:
    return x + 1
