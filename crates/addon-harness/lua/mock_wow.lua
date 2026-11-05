-- A small fake of the WoW API surface WowTracker uses, for tests.
-- "Secret" values (Forever/Midnight) are a table: comparing them with < or >
-- or doing maths on them raises an error, much like the real client.

Mock = {
  clock = 1730000000,
  frames = {},
  printed = {},
  inCombat = false,
  reloadRequested = false,
  player = {
    name = "Testchar", realm = "Firemaw", level = 10, xp = 500, xpMax = 7000, rested = 1200,
    resting = false, className = "Warrior", classFile = "WARRIOR", race = "Human", faction = "Alliance",
  },
  map = { id = 1429, x = 0.4123456, y = 0.6, zone = "Elwynn Forest", subzone = "Goldshire", instance = nil },
  target = nil,
}

local SECRET = setmetatable({}, { __tostring = function() return "<secret>" end })
Mock.SECRET = SECRET

function issecretvalue(v) return v == SECRET end

-- NPC identity and health are hidden in combat.
local function hidden(v)
  if Mock.inCombat and Mock.target and not Mock.target.isPlayer then return SECRET end
  return v
end

-- Frames ---------------------------------------------------------------------

local Frame = {}
local FrameMT = {
  __index = function(self, k)
    local m = Frame[k]
    if m then return m end
    return function() return self end -- any other widget method: no-op
  end,
}

function Frame:RegisterEvent(e) self.events[e] = true end
function Frame:RegisterUnitEvent(e) self.events[e] = true end
function Frame:UnregisterEvent(e) self.events[e] = nil end
function Frame:SetScript(name, fn) self.scripts[name] = fn end
function Frame:GetScript(name) return self.scripts[name] end

function CreateFrame()
  local f = setmetatable({ events = {}, scripts = {} }, FrameMT)
  table.insert(Mock.frames, f)
  return f
end

function Mock.fire(event, ...)
  for _, f in ipairs(Mock.frames) do
    if f.events[event] and f.scripts.OnEvent then f.scripts.OnEvent(f, event, ...) end
  end
end

function Mock.update(seconds)
  Mock.clock = Mock.clock + seconds
  for _, f in ipairs(Mock.frames) do
    if f.scripts.OnUpdate then f.scripts.OnUpdate(f, seconds) end
  end
end

-- API ------------------------------------------------------------------------

function GetServerTime() return Mock.clock end
function time() return Mock.clock end
function print(...)
  local parts = {}
  for i = 1, select("#", ...) do parts[#parts + 1] = tostring((select(i, ...))) end
  table.insert(Mock.printed, table.concat(parts, " "))
end

function hooksecurefunc(a, b, c)
  local tbl, name, fn
  if type(a) == "table" then tbl, name, fn = a, b, c else tbl, name, fn = _G, a, b end
  local orig = tbl[name]
  tbl[name] = function(...)
    local r = { orig(...) }
    fn(...)
    return unpack(r)
  end
end

C_AddOns = { GetAddOnMetadata = function(_, field) if field == "Version" then return "0.1.0" end end }
C_UI = { Reload = function() Mock.reloadRequested = true end }
function ReloadUI() C_UI.Reload() end

Settings = {
  RegisterCanvasLayoutCategory = function(_, name) return { name = name, GetID = function() return 42 end } end,
  RegisterAddOnCategory = function() end,
  OpenToCategory = function(id) Mock.openedCategory = id end,
}
SlashCmdList = {}

C_Map = {
  GetBestMapForUnit = function() if Mock.map.instance then return nil end return Mock.map.id end,
  GetPlayerMapPosition = function()
    if Mock.map.instance then return nil end
    return { GetXY = function() return Mock.map.x, Mock.map.y end }
  end,
}

function GetRealZoneText() return Mock.map.zone end
function GetSubZoneText() return Mock.map.subzone end
function IsInInstance() return Mock.map.instance ~= nil, Mock.map.instance or "none" end
function GetRealmName() return Mock.player.realm end

local function T() return Mock.target end

function UnitExists(unit)
  if unit == "player" then return true end
  return unit == "target" and T() ~= nil
end

function UnitName(unit)
  if unit == "player" then return Mock.player.name, nil end
  return hidden(T().name), nil
end

function UnitLevel(unit)
  if unit == "player" then return Mock.player.level end
  return T().level
end

function UnitXP() return Mock.player.xp end
function UnitXPMax() return Mock.player.xpMax end
function GetXPExhaustion() if Mock.player.rested > 0 then return Mock.player.rested end return nil end
function IsResting() return Mock.player.resting end
function UnitClass() return Mock.player.className, Mock.player.classFile end
function UnitGUID(unit)
  if unit == "player" then return Mock.player.guid or "Player-6940-00AF5B76" end
  return nil
end
function UnitRace() return Mock.player.race, Mock.player.race end
function UnitFactionGroup() return Mock.player.faction end
function UnitAffectingCombat() return Mock.inCombat end
function UnitClassification() return T().classification end
function UnitCreatureType() return hidden(T().creatureType) end
function UnitReaction() return T().reaction end
function UnitIsPlayer() return T().isPlayer and true or false end
function UnitIsDead() return T().hp == 0 end
function UnitHealth() return hidden(T().hp) end
function UnitHealthMax() return hidden(T().hpMax) end

-- SavedVariables writer, imitating the client's format ------------------------

local function isint(v) return v == math.floor(v) and math.abs(v) < 2 ^ 53 end

local function ser(v, out)
  local t = type(v)
  if t == "string" then
    out[#out + 1] = string.format("%q", v)
  elseif t == "number" then
    out[#out + 1] = isint(v) and string.format("%d", v) or string.format("%.14g", v)
  elseif t == "boolean" then
    out[#out + 1] = tostring(v)
  elseif t == "table" then
    if v == SECRET then error("a secret value reached SavedVariables") end
    out[#out + 1] = "{\n"
    local n = #v
    for i = 1, n do
      ser(v[i], out)
      out[#out + 1] = ", -- [" .. i .. "]\n"
    end
    local keys = {}
    for k in pairs(v) do
      if not (type(k) == "number" and k >= 1 and k <= n and isint(k)) then keys[#keys + 1] = k end
    end
    table.sort(keys, function(a, b) return tostring(a) < tostring(b) end)
    for _, k in ipairs(keys) do
      if type(k) == "string" then out[#out + 1] = string.format("[%q] = ", k)
      else out[#out + 1] = "[" .. k .. "] = " end
      ser(v[k], out)
      out[#out + 1] = ",\n"
    end
    out[#out + 1] = "}"
  else
    out[#out + 1] = "nil"
  end
end

function Mock.saved_variables()
  local out = { "\nWowTrackerDB = " }
  ser(WowTrackerDB, out)
  out[#out + 1] = "\n"
  return table.concat(out)
end
