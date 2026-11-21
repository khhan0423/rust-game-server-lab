# Shared helpers; callers run from the workspace root.
load_env() {
    [[ -f .env ]] || return 0
    local line key value
    while IFS= read -r line || [[ -n $line ]]; do
        [[ $line =~ ^([a-zA-Z_][a-zA-Z0-9_]*)=(.*)$ ]] || continue
        key=${BASH_REMATCH[1]} value=${BASH_REMATCH[2]}
        [[ ! -v $key ]] || continue
        value=${value%$'\r'}
        if [[ $value == \"*\" || $value == \'*\' ]]; then value=${value:1:-1}; fi
        export "$key=$value"
    done < .env
}
resolve_binary() {
    local binary="${CARGO_TARGET_DIR:-target}/debug/$1"
    [[ ! -f $binary.exe ]] || binary=$binary.exe
    printf '%s\n' "$binary"
}
config_for() {
    local key=${1^^}_CONFIG
    printf '%s\n' "${!key:-dev/config/$1.toml}"
}
build_unless_prebuilt() {
    [[ ${RUN_PREBUILT:-0} == 1 ]] || SQLX_OFFLINE=true cargo build "$@"
}
