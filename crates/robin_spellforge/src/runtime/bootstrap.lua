
local base = _G
local private_coroutine = coroutine
local private_sethook = debug.sethook
local private_setfenv = setfenv
local modules = __robin_modules
local aliases = __robin_module_aliases
local entry = __robin_entry
local names = __robin_names
local instruction_limit = __robin_instruction_limit
local script = {}
script._G = script
local module_cache = {}
local sequence_callbacks = { __next_id = 10000 }
local activations = {}
local budget_hook = function() error('Spellforge execution budget exceeded', 0) end

local function private_require(name)
    if type(name) ~= 'string' then error('require expects a module name string', 2) end
    local canonical = aliases[name]
    if canonical == nil then error("Spellforge package has no module '" .. name .. "'", 2) end
    if module_cache[canonical] ~= nil then return module_cache[canonical] end
    local loader = modules[canonical]
    if type(loader) ~= 'function' then error("Spellforge module '" .. canonical .. "' has no loader", 2) end
    private_setfenv(loader, script)
    local result = loader(canonical)
    if result == nil then result = true end
    module_cache[canonical] = result
    return result
end
script.require = private_require
setmetatable(script, { __index = base, __metatable = false })

local function by_name(kind, name) return names[kind][name] or 0 end
base.GetActor = function(name) return by_name('actors', name) end
base.GetItem = function(name) return by_name('items', name) end
base.GetLocation = function(name) return by_name('locations', name) end
base.GetPatrol = function(name) return by_name('patrols', name) end
base.GetScroll = function(name) return by_name('scrolls', name) end
base.GetActorName = function(handle)
    for name, value in pairs(names.actors) do if value == handle then return name end end
    return '<not found>'
end
base.GetAllActors = function()
    local result = {}
    for name, handle in pairs(names.actors) do result[name] = handle end
    return result
end
base.SequenceCall = function(callback)
    if type(callback) ~= 'function' then error('SequenceCall expects a function', 2) end
    local id = sequence_callbacks.__next_id
    sequence_callbacks.__next_id = id + 1
    sequence_callbacks[id] = callback
    return SequenceSendMessage(0, id)
end

local random_yield = private_coroutine.yield
math.random = function(...)
    local count = select('#', ...)
    if count > 2 then error('math.random: expected 0..=2 arguments', 2) end
    local args = {...}
    for i = 1, count do
        local value = args[i]
        if type(value) ~= 'number' or value ~= math.floor(value) or value < -2147483648 or value > 2147483647 then
            error('math.random: bounds must be signed 32-bit integers', 2)
        end
    end
    if count == 1 and args[1] < 1 then error('math.random: upper bound must be >= 1', 2) end
    if count == 2 and args[1] > args[2] then error('math.random: empty interval', 2) end
    local word = random_yield('__robin_native_v1', 4294967295, args)
    if count == 0 then return __robin_unpack_f32(word) end
    return word
end
math.randomseed = function(...) end

local function resolve(kind, class, event, message)
    if kind == 0 then
        if event == 'ProcessMessage' and message ~= nil and rawget(sequence_callbacks, message) ~= nil then
            return rawget(sequence_callbacks, message)
        end
        return rawget(script, event)
    end
    local class_table = rawget(script, class)
    if class_table == nil then return nil end
    if type(class_table) ~= 'table' then return false, 'class' end
    return rawget(class_table, event)
end
local function has_handler(kind, class, event, message)
    local handler, problem = resolve(kind, class, event, message)
    if handler == nil then return 0 end
    if problem ~= nil or type(handler) ~= 'function' then return 2 end
    return 1
end
local function drive(id, co, ...)
    local values = { private_coroutine.resume(co, ...) }
    if not values[1] then error(values[2], 0) end
    table.remove(values, 1)
    if private_coroutine.status(co) == 'dead' then
        activations[id] = nil
        return true, unpack(values)
    end
    return false, unpack(values)
end
local function begin(id, kind, class, event, message, ...)
    local handler, problem = resolve(kind, class, event, message)
    if handler == nil then error('Spellforge handler disappeared before dispatch', 0) end
    if problem ~= nil or type(handler) ~= 'function' then error('Spellforge handler is not a function', 0) end
    local co = private_coroutine.create(handler)
    private_sethook(co, budget_hook, '', instruction_limit)
    activations[id] = co
    return drive(id, co, ...)
end
local function resume(id, word)
    local co = activations[id]
    if co == nil then error('unknown Spellforge activation ' .. tostring(id), 0) end
    return drive(id, co, word)
end

private_setfenv(entry, script)
-- The driver has already captured the few privileged operations it needs.
-- Remove them before mission top-level code runs so a package cannot retain
-- a capability and escape the per-package environment later.
base.__robin_modules = nil
base.__robin_module_aliases = nil
base.__robin_entry = nil
base.__robin_names = nil
base.__robin_instruction_limit = nil
base.__robin_pack_f32 = nil
base.__robin_unpack_f32 = nil
base.coroutine = nil
base.debug = nil
base.io = nil
base.os = nil
base.package = nil
base.dofile = nil
base.load = nil
base.loadfile = nil
base.loadstring = nil
base.getfenv = nil
base.setfenv = nil
base.collectgarbage = nil
base._G = nil

private_sethook(budget_hook, '', instruction_limit)
entry()
private_sethook()

base.__robin_driver = { has_handler = has_handler, begin = begin, resume = resume }
