-- WoW Tracker: live strip
--
-- Draws a row of small coloured squares in the top-left corner of the game
-- window. Each square carries three bytes (its red, green and blue values).
-- The WowTracker companion reads them off the screen about 20 times a second,
-- which is how the website updates within a second instead of waiting for a
-- /reload. Turn it off with "/wt strip off".
--
-- Frame layout (bytes; numbers are big-endian):
--   0-1   "WT" marker
--   2     format version (1)
--   3     sequence number, +1 every redraw
--   4     flags: 1 in combat, 2 dead, 4 ghost, 8 resting, 16 has position
--   5     level
--   6-8   XP             9-11 XP to level      12-14 rested XP
--   15-16 uiMapID        17-18 x * 65535       19-20 y * 65535
--   21-24 copper         25-28 server time (Unix seconds)
--   29    message ID (0 = none)   30 chunk index   31 chunk count   32 chunk length
--   33-90 chunk bytes (58)
--   91-92 Fletcher-16 checksum of bytes 0-90
--
-- Messages are a type letter followed by JSON: I identity, Z zone, Q quest log,
-- E journal entry. Each chunk stays on screen for two redraws so a reader at
-- 20 frames per second sees every one.

local ADDON_NAME, ns = ...

local VERSION = 1
local CELL = 3              -- physical pixels per square
local CHUNK = 58
local FRAME_BYTES = 93      -- 31 squares
local REDRAW = 0.1          -- seconds between redraws
local HOLD = 2              -- redraws per chunk
local IDENTITY_EVERY = 30   -- seconds
local ZONE_EVERY = 15
local QUESTS_EVERY = 60

local strip = {}
ns.strip = strip

local frame, cells
local seq = 0
local queue = {}            -- pending messages: { kind, bytes }
local current               -- message being sent: { id, bytes, count, index, shown }
local nextID = 0
local lastSent = {}         -- kind -> { at, text }

---------------------------------------------------------------------------
-- JSON (just what the messages need)
---------------------------------------------------------------------------

local function jsonString(s)
  return '"' .. s:gsub('[%c"\\]', function(c)
    if c == '"' then return '\\"' elseif c == "\\" then return "\\\\"
    elseif c == "\n" then return "\\n" elseif c == "\t" then return "\\t" end
    return string.format("\\u%04x", c:byte())
  end) .. '"'
end

local function json(v)
  local t = type(v)
  if t == "string" then return jsonString(v)
  elseif t == "number" then
    if v ~= v or v == math.huge or v == -math.huge then return "null" end
    if v == math.floor(v) and math.abs(v) < 2^53 then return string.format("%d", v) end
    return string.format("%.4f", v)
  elseif t == "boolean" then return v and "true" or "false"
  elseif t == "table" then
    local out = {}
    if #v > 0 or next(v) == nil then
      for i = 1, #v do out[i] = json(v[i]) end
      return "[" .. table.concat(out, ",") .. "]"
    end
    local keys = {}
    for k in pairs(v) do if type(k) == "string" then keys[#keys + 1] = k end end
    table.sort(keys)
    for _, k in ipairs(keys) do
      local val = v[k]
      local vt = type(val)
      if vt == "string" or vt == "number" or vt == "boolean" or vt == "table" then
        out[#out + 1] = jsonString(k) .. ":" .. json(val)
      end
    end
    return "{" .. table.concat(out, ",") .. "}"
  end
  return "null"
end
strip.json = json

---------------------------------------------------------------------------
-- Messages
---------------------------------------------------------------------------

local function enqueue(kind, payload)
  local text = json(payload)
  local bytes = kind .. text
  if #bytes > CHUNK * 255 then return end
  if kind ~= "E" then
    -- Only the newest identity, zone or quest log matters.
    for i = #queue, 1, -1 do
      if queue[i].kind == kind then table.remove(queue, i) end
    end
  end
  lastSent[kind] = { at = GetTime and GetTime() or 0, text = text }
  local item = { kind = kind, bytes = bytes }
  if kind == "E" then
    -- Journal entries go first.
    local at = 1
    while queue[at] and queue[at].kind == "E" do at = at + 1 end
    table.insert(queue, at, item)
  else
    queue[#queue + 1] = item
  end
end

-- Sends `payload` when it changed, or anyway every `every` seconds.
local function offer(kind, payload, every)
  local last = lastSent[kind]
  local t = GetTime and GetTime() or 0
  if last and t - last.at < every and last.text == json(payload) then return end
  enqueue(kind, payload)
end

function strip.journal(entry)
  if strip.enabled() then enqueue("E", entry) end
end

local function nextChunk()
  if current and current.shown < HOLD then
    current.shown = current.shown + 1
    return current
  end
  if current and current.index < current.count then
    current.index = current.index + 1
    current.shown = 1
    return current
  end
  current = nil
  local item = table.remove(queue, 1)
  if not item then return nil end
  nextID = nextID % 255 + 1
  current = { id = nextID, bytes = item.bytes, count = math.max(1, math.ceil(#item.bytes / CHUNK)), index = 1, shown = 1 }
  return current
end

---------------------------------------------------------------------------
-- Frame bytes
---------------------------------------------------------------------------

local function put(buf, n, size)
  n = math.max(0, math.floor(tonumber(n) or 0))
  local max = 256 ^ size
  if n >= max then n = max - 1 end
  for i = size - 1, 0, -1 do
    buf[#buf + 1] = math.floor(n / 256 ^ i) % 256
  end
end

local function playerPosition()
  if not (C_Map and C_Map.GetBestMapForUnit) then return nil end
  local mapID = ns.plain(C_Map.GetBestMapForUnit("player"))
  if not mapID then return nil end
  local pos = C_Map.GetPlayerMapPosition(mapID, "player")
  if not pos then return nil end
  local x, y = pos:GetXY()
  x, y = ns.plain(x), ns.plain(y)
  if not (x and y) or (x == 0 and y == 0) then return nil end
  return mapID, x, y
end

function strip.build(char)
  local b = {}
  b[1], b[2], b[3] = 87, 84, VERSION -- "WT"
  seq = (seq + 1) % 256
  b[4] = seq
  local mapID, x, y = playerPosition()
  local flags = 0
  if char.inCombat then flags = flags + 1 end
  if UnitIsDead and ns.plain(UnitIsDead("player")) then flags = flags + 2 end
  if UnitIsGhost and ns.plain(UnitIsGhost("player")) then flags = flags + 4 end
  if char.resting then flags = flags + 8 end
  if mapID then flags = flags + 16 end
  b[5] = flags
  b[6] = math.min(255, char.level or 0)
  put(b, char.xp, 3); put(b, char.xpMax, 3); put(b, char.rested, 3)
  put(b, mapID or char.mapID, 2)
  put(b, (x or 0) * 65535 + 0.5, 2); put(b, (y or 0) * 65535 + 0.5, 2)
  put(b, char.money, 4)
  put(b, ns.now(), 4)

  local c = nextChunk()
  if c then
    local from = (c.index - 1) * CHUNK + 1
    local part = c.bytes:sub(from, from + CHUNK - 1)
    b[#b + 1], b[#b + 2], b[#b + 3], b[#b + 4] = c.id, c.index - 1, c.count, #part
    for i = 1, CHUNK do b[#b + 1] = part:byte(i) or 0 end
  else
    for _ = 1, 4 + CHUNK do b[#b + 1] = 0 end
  end

  -- Fletcher-16 over everything so far.
  local s1, s2 = 0, 0
  for i = 1, #b do
    s1 = (s1 + b[i]) % 255
    s2 = (s2 + s1) % 255
  end
  b[#b + 1], b[#b + 2] = s1, s2
  return b
end

---------------------------------------------------------------------------
-- Drawing
---------------------------------------------------------------------------

local function layout()
  if not frame then return end
  local _, h = 0, 768
  if GetPhysicalScreenSize then _, h = GetPhysicalScreenSize() end
  if not h or h <= 0 then h = 768 end
  -- One unit = one physical pixel. The frame has no parent, so the UI scale
  -- setting does not apply and it stays visible when the UI is hidden (Alt+Z).
  frame:SetScale(768 / h)
  frame:ClearAllPoints()
  frame:SetPoint("TOPLEFT", nil, "TOPLEFT", 0, 0)
  frame:SetSize(CELL * FRAME_BYTES / 3, CELL)
  for i, t in ipairs(cells) do
    t:ClearAllPoints()
    t:SetPoint("TOPLEFT", frame, "TOPLEFT", (i - 1) * CELL, 0)
    t:SetSize(CELL, CELL)
  end
end

local function create()
  frame = CreateFrame("Frame", "WowTrackerStrip", nil)
  frame:SetFrameStrata("TOOLTIP")
  frame:SetFrameLevel(10000)
  cells = {}
  for i = 1, FRAME_BYTES / 3 do
    local t = frame:CreateTexture(nil, "OVERLAY")
    t:SetColorTexture(0, 0, 0, 1)
    pcall(t.SetSnapToPixelGrid, t, false)
    pcall(t.SetTexelSnappingBias, t, 0)
    cells[i] = t
  end
  layout()
end

local function draw(bytes)
  for i, t in ipairs(cells) do
    local j = (i - 1) * 3
    t:SetColorTexture(bytes[j + 1] / 255, bytes[j + 2] / 255, bytes[j + 3] / 255, 1)
  end
end

function strip.enabled()
  local db = ns.db()
  return db and db.settings.strip and not db.settings.appearOffline
end

local since, offeredAt = 0, -1
local function onUpdate(_, elapsed)
  since = since + elapsed
  if since < REDRAW then return end
  since = 0
  local char = ns.char()
  if not char then return end
  if not strip.enabled() then frame:Hide(); return end
  frame:Show()

  -- Look for changed identity, zone and quests once a second.
  local t = GetTime and GetTime() or 0
  if t - offeredAt >= 1 then
    offeredAt = t
    offer("I", {
      key = char.key, name = char.name, realm = char.realm, guid = char.guid,
      class = char.class, classFile = char.classFile, race = char.race, faction = char.faction,
    }, IDENTITY_EVERY)
    offer("Z", { zone = char.zone or "", subzone = char.subzone or "", instance = char.instance }, ZONE_EVERY)
    if char.quests then offer("Q", char.quests, QUESTS_EVERY) end
  end

  draw(strip.build(char))
end

function strip.setEnabled(on)
  ns.db().settings.strip = on
  if frame then if on then frame:Show() else frame:Hide() end end
end

function strip.start()
  if not frame then create() end
  frame:SetScript("OnUpdate", onUpdate)
  frame:RegisterEvent("DISPLAY_SIZE_CHANGED")
  frame:RegisterEvent("UI_SCALE_CHANGED")
  frame:SetScript("OnEvent", layout)
end
