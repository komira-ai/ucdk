# The part of every resource template that is the same for each kind. The kinds follow.


def _use(target: Any) -> ucdk_resource_v1.Uses:
    """A `Uses` from a `Uses`, a `Ref`, or a template (anything with a `ref`)."""
    if isinstance(target, ucdk_resource_v1.Uses):
        return target
    ref = target if isinstance(target, ucdk_resource_v1.Ref) else target.ref
    return ucdk_resource_v1.Uses(target=ref, access=@ACCESS@)


def _value(item: Any) -> ucdk_resource_v1.Value:
    """A `Value` from a `Value`, a literal string, or a `Ref` to another resource's output."""
    if isinstance(item, ucdk_resource_v1.Value):
        return item
    if isinstance(item, str):
        return ucdk_resource_v1.Value(literal=item)
    return ucdk_resource_v1.Value(ref=item)


def _values(items: Optional[Mapping[str, Any]]) -> Optional[Dict[str, ucdk_resource_v1.Value]]:
    return None if items is None else {key: _value(item) for key, item in items.items()}


class _Template:
    """An id, what the resource uses, and its body; `to_resource` makes the wire message."""

    KIND = ""

    def __init__(self, id: str, uses: Iterable[Any]) -> None:
        self.id = id
        self.uses = list(uses)
        self.body: Any = None

    @property
    def ref(self) -> ucdk_resource_v1.Ref:
        """A reference to this resource itself, as `uses` takes it."""
        return ucdk_resource_v1.Ref(resource=self.id)

    def to_resource(self) -> ucdk_resource_v1.Resource:
        return ucdk_resource_v1.Resource(
            id=self.id, uses=[_use(u) for u in self.uses], **{self.KIND: self.body}
        )


def resource_list(*resources: Any) -> ucdk_resource_v1.ResourceList:
    """The `ResourceList` of templates (or already-built `Resource` messages)."""
    return ucdk_resource_v1.ResourceList(
        resource=[
            r if isinstance(r, ucdk_resource_v1.Resource) else r.to_resource() for r in resources
        ]
    )
