local Runtime = {}
local hiddenValue = "ignore"

function Runtime.connect(target: string): boolean
    return target ~= ""
end

function Runtime:refresh(token: string): string
    return token
end

export type Session = {
    token: string,
}

function leak(): string
    return hiddenValue
end

return Runtime
