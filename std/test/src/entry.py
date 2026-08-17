# Registry of (description, func, fixture_names) populated by @test.
_tests: list[tuple[str, object, tuple[str, ...]]] = []
_fixtures: dict[str, object] = {}

def fixture(func):
    _fixtures[func.__name__] = func
    return func

def test(description: str, *uses: str):
    """
    Register a test
        `uses` names fixtures injected by keyword.
    """
    def decorator(func):
        _tests.append((description, func, uses))
        return func
    return decorator

class Raises:
    def __init__(self, exc_type: type) -> None:
        self.exc_type = exc_type

    def __enter__(self):
        return self

    def __exit__(self, etype, _exc, _tb) -> bool:
        if etype is None:
            raise AssertionError(f"Expected {self.exc_type.__name__}, nothing was raised")
        # Swallow the exception only if it matches the expected type.
        return issubclass(etype, self.exc_type)

def raises(exc_type: type) -> Raises:
    return Raises(exc_type)

def _build_kwargs(uses: tuple[str, ...]) -> dict:
    kwargs = {}
    for name in uses:
        if name not in _fixtures:
            raise KeyError(f"Unknown fixture: {name!r}")
        kwargs[name] = _fixtures[name]()  # call fixture fresh per test
    return kwargs

def run() -> None:
    passed = 0
    failed = 0
    for description, func, uses in _tests:
        try:
            func(**_build_kwargs(uses))
            print(f"PASS - {description}")
            passed += 1
        except AssertionError as e:  # test failed
            print(f"FAIL - {description} (AssertionError: {e})")
            failed += 1
        except Exception as e:  # unexpected error in the test
            print(f"ERROR - {description} ({type(e).__name__}: {e})")
            failed += 1
    print(f"{passed} passed, {failed} failed")
    raise SystemExit(1 if failed else 0)

if __name__ == "__main__":
    @fixture
    def user() -> dict:
        return {"name": "Ana", "age": 30}

    @test("user has a valid name", "user")
    def test_name(user: dict) -> None:
        assert user["name"] == "Ana"

    @test("2 + 3 equals 5")
    def test_sum() -> None:
        a, b, expected = 2, 3, 5
        assert a + b == expected

    @test("dividing by zero raises an error")
    def test_div() -> None:
        with raises(ZeroDivisionError):
            1 / 0

    @test("this one fails on purpose")
    def test_fail() -> None:
        assert 1 == 2, "1 != 2"

    run()
