// The part of every resource template that is the same for each kind. The kinds follow.

/** Anything `uses` takes: a `Uses`, a `Ref`, or a template (anything with a `ref`). */
export type UseInit = ucdk_resource_v1.Uses | ucdk_resource_v1.Ref | Template;

/** Anything a configuration value takes: a `Value`, a literal string, or a `Ref` to another
 * resource's output. */
export type ValueInit = ucdk_resource_v1.Value | string | ucdk_resource_v1.Ref;

function use(target: UseInit): ucdk_resource_v1.Uses {
  if (target instanceof ucdk_resource_v1.Uses) {
    return target;
  }
  const ref = target instanceof ucdk_resource_v1.Ref ? target : target.ref;
  return new ucdk_resource_v1.Uses({ target: ref, access: @ACCESS@ });
}

function value(item: ValueInit): ucdk_resource_v1.Value {
  if (item instanceof ucdk_resource_v1.Value) {
    return item;
  }
  if (typeof item === "string") {
    return new ucdk_resource_v1.Value({ literal: item });
  }
  return new ucdk_resource_v1.Value({ ref: item });
}

function values(
  items: wire.MapInit<ValueInit> | undefined,
): Map<string, ucdk_resource_v1.Value> | undefined {
  if (items === undefined) {
    return undefined;
  }
  const out = new Map<string, ucdk_resource_v1.Value>();
  for (const [key, item] of wire.toMap(items)) {
    out.set(key, value(item));
  }
  return out;
}

/** An id, what the resource uses, and its body; `toResource` makes the wire message. */
export abstract class Template {
  readonly id: string;
  readonly uses: UseInit[];

  protected constructor(id: string, uses: Iterable<UseInit>) {
    this.id = id;
    this.uses = Array.from(uses);
  }

  /** A reference to this resource itself, as `uses` takes it. */
  get ref(): ucdk_resource_v1.Ref {
    return new ucdk_resource_v1.Ref({ resource: this.id });
  }

  abstract toResource(): ucdk_resource_v1.Resource;

  protected usesAsWire(): ucdk_resource_v1.Uses[] {
    return this.uses.map(use);
  }
}

/** The `ResourceList` of templates (or already-built `Resource` messages). */
export function resourceList(
  ...resources: Array<Template | ucdk_resource_v1.Resource>
): ucdk_resource_v1.ResourceList {
  return new ucdk_resource_v1.ResourceList({
    resource: resources.map((r) => (r instanceof Template ? r.toResource() : r)),
  });
}
