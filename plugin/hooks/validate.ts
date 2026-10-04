/** The subset of JSON Schema the account's schemas use (see `profiles.ts`). */
export type Schema = {
  type?: 'object' | 'array' | 'string' | 'boolean' | 'integer'
  enum?: readonly unknown[]
  properties?: Readonly<Record<string, Schema>>
  required?: readonly string[]
  additionalProperties?: boolean
  items?: Schema
}

const kinds: Record<NonNullable<Schema['type']>, (value: unknown) => boolean> = {
  object: value => typeof value === 'object' && value !== null && !Array.isArray(value),
  array: Array.isArray,
  string: value => typeof value === 'string',
  boolean: value => typeof value === 'boolean',
  integer: Number.isInteger,
}

/** Where `value` breaks `schema`, as `$.s.c[0].l: not a boolean`; undefined when it fits. */
export function validate(value: unknown, schema: Schema, path = '$'): string | undefined {
  if (schema.type !== undefined && !kinds[schema.type](value)) {
    return `${path}: not ${schema.type === 'array' || schema.type === 'object' ? 'an' : 'a'} ${schema.type}`
  }
  if (schema.enum !== undefined && !schema.enum.includes(value)) {
    return `${path}: not one of ${schema.enum.map(word => JSON.stringify(word)).join(', ')}`
  }
  if (Array.isArray(value) && schema.items !== undefined) {
    for (const [index, item] of value.entries()) {
      const error = validate(item, schema.items, `${path}[${index}]`)
      if (error !== undefined) {
        return error
      }
    }
  }
  if (schema.properties !== undefined && typeof value === 'object' && value !== null) {
    const fields = value as Record<string, unknown>
    for (const name of schema.required ?? []) {
      if (!(name in fields)) {
        return `${path}: missing "${name}"`
      }
    }
    for (const [name, field] of Object.entries(fields)) {
      const inner = schema.properties[name]
      if (inner === undefined) {
        if (schema.additionalProperties === false) {
          return `${path}: unknown key "${name}"`
        }
        continue
      }
      const error = validate(field, inner, `${path}.${name}`)
      if (error !== undefined) {
        return error
      }
    }
  }
  return undefined
}
