-- WoW Tracker
--
-- Records your character's position, level, XP and combat state into
-- SavedVariables (WowTrackerDB). The WowTracker companion app watches that
-- file and sends it to your backend.
--
-- Addons cannot open network connections or write files. The game only writes
-- SavedVariables to disk on /reload, logout or exit, so that is when the
-- website updates. "/wt sync" saves right away (it reloads the UI).
--
-- Settings (/wt config, or the slash commands below) are stored in the same
-- file, which is how the companion learns where to send the data.

local ADDON_NAME = ...
local SCHEMA = 1

local POSITION_INTERVAL = 1   -- seconds between position reads
local TRAIL_INTERVAL = 10     -- minimum seconds between trail points
local TRAIL_MIN_MOVE = 0.003  -- minimum movement (fraction of the zone map) for a new trail point
local TRAIL_MAX = 1500        -- trail points kept per character
local JOURNAL_MAX = 1000      -- journal entries kept per character
local XP_SAMPLE_MAX = 2000    -- levelling-curve samples kept per character
local XP_SAMPLE_INTERVAL = 60 -- minimum played seconds between samples at the same level
local QUEST_REFRESH = 1       -- minimum seconds between quest log reads

local DEFAULT_SETTINGS = {
  endpoint = "",  -- backend ingest address, e.g. "192.168.0.117:1337" or "https://ingest.example.com"
  token = "",     -- shared secret the backend expects (optional)
  trail = true,   -- record a breadcrumb trail of where you have been
}

local VERSION = (C_AddOns and C_AddOns.GetAddOnMetadata and C_AddOns.GetAddOnMetadata(ADDON_NAME, "Version")) or "dev"

local db         -- WowTrackerDB (account-wide)
local char       -- db.characters[<Name-Realm>]
local reloading = false
local session = {}
local optionsCategory

---------------------------------------------------------------------------
-- Helpers
---------------------------------------------------------------------------

local function now()
  return (GetServerTime and GetServerTime()) or time()
end

-- Forever (like Midnight) hides some values from addons as "secret values".
-- They cannot be compared, used in maths or meaningfully saved, so drop them.
local function plain(v)
  if issecretvalue and issecretvalue(v) then return nil end
  return v
end

local function round(v, places)
  local m = 10 ^ places
  return math.floor(v * m + 0.5) / m
end

local function trim(s)
  return ((s or ""):gsub("^%s+", ""):gsub("%s+$", ""))
end

local function say(msg)
  print("|cff4fc3f7WoW Tracker:|r " .. msg)
end

---------------------------------------------------------------------------
-- Data collection
---------------------------------------------------------------------------

local function addTrailPoint(mapID, x, y)
  if not db.settings.trail then return end
  local trail = char.trail
  local t = now()
  local last = trail[#trail]
  if last then
    if t - last[1] < TRAIL_INTERVAL then return end
    if last[2] == mapID and math.abs(last[3] - x) < TRAIL_MIN_MOVE and math.abs(last[4] - y) < TRAIL_MIN_MOVE then
      return
    end
  end
  trail[#trail + 1] = { t, mapID, x, y }
  while #trail > TRAIL_MAX do table.remove(trail, 1) end
end

-- Journal of levels, quests, zones, deaths and logins, newest last.
local function journal(kind, entry)
  entry = entry or {}
  entry.t = now()
  entry.kind = kind
  local j = char.journal
  j[#j + 1] = entry
  while #j > JOURNAL_MAX do table.remove(j, 1) end
end

-- Played time in seconds. Counted while logged in, and corrected whenever the
-- game reports the real total (TIME_PLAYED_MSG, e.g. after /played).
local function onTimePlayed(total)
  total = plain(total)
  if not (char and type(total) == "number") then return end
  if not char.playedSynced then
    -- First real figure: shift what was counted before it.
    local offset = total - (char.played or 0)
    for _, s in ipairs(char.xpSamples) do s[1] = s[1] + offset end
    char.playedSynced = true
  end
  char.played = total
end

-- Levelling curve and XP/hour: { played, level, xp, xpMax } samples.
local function sampleXP(force)
  if not (char and char.level and char.xpMax) then return end
  local samples = char.xpSamples
  local played = math.floor(char.played or 0)
  local last = samples[#samples]
  if last and not force then
    if last[2] == char.level and (last[3] == char.xp or played - last[1] < XP_SAMPLE_INTERVAL) then return end
  end
  if last and last[1] == played then table.remove(samples) end
  samples[#samples + 1] = { played, char.level, char.xp, char.xpMax }
  while #samples > XP_SAMPLE_MAX do table.remove(samples, 1) end
end

-- Health and power, when the game does not hide them.
local function updateVitals()
  if not char then return end
  char.hp, char.hpMax = plain(UnitHealth("player")), plain(UnitHealthMax("player"))
  if UnitPower then
    char.powerType = plain((UnitPowerType("player")))
    char.power, char.powerMax = plain(UnitPower("player")), plain(UnitPowerMax("player"))
  end
end

local function updateMoney()
  if char and GetMoney then char.money = plain(GetMoney()) end
end

-- Quest log ----------------------------------------------------------------

-- "Vial of Animal Blood: 4/6" or "4/6 Vial of Animal Blood" -> text, have, need.
local function objective(text, have, need, done)
  text = plain(text)
  if type(text) ~= "string" or text == "" then return nil end
  local name, a, b = text:match("^(.-):%s*(%d+)%s*/%s*(%d+)%s*$")
  if not name then
    a, b, name = text:match("^(%d+)%s*/%s*(%d+)%s+(.+)$")
  end
  if name then
    text = name
    have = plain(have) or tonumber(a)
    need = plain(need) or tonumber(b)
  end
  return { text = text, have = plain(have), need = plain(need), done = plain(done) and true or false }
end

local function readObjectives(questID, index)
  local out = {}
  local list = questID and C_QuestLog and C_QuestLog.GetQuestObjectives and C_QuestLog.GetQuestObjectives(questID)
  if type(list) == "table" then
    for _, o in ipairs(list) do
      out[#out + 1] = objective(o.text, o.numFulfilled, o.numRequired, o.finished)
    end
  elseif index and GetNumQuestLeaderBoards and GetQuestLogLeaderBoard then
    for j = 1, GetNumQuestLeaderBoards(index) or 0 do
      local text, _, finished = GetQuestLogLeaderBoard(j, index)
      out[#out + 1] = objective(text, nil, nil, finished)
    end
  end
  return out
end

-- One quest log row: title, level, isHeader, complete, failed, questID.
local function questLogRow(i)
  if GetQuestLogTitle then
    local title, level, _, isHeader, _, complete, _, questID = GetQuestLogTitle(i)
    return title, level, isHeader, complete == 1, complete == -1, questID
  end
  local info = C_QuestLog.GetInfo(i)
  if not info or info.isHidden then return nil end
  local done = info.questID and C_QuestLog.IsComplete and C_QuestLog.IsComplete(info.questID)
  local failed = info.questID and C_QuestLog.IsFailed and C_QuestLog.IsFailed(info.questID)
  return info.title, info.level, info.isHeader, done, failed, info.questID
end

local function readQuestLog()
  if not char then return end
  local count
  if GetQuestLogTitle and GetNumQuestLogEntries then
    count = GetNumQuestLogEntries()
  elseif C_QuestLog and C_QuestLog.GetInfo and C_QuestLog.GetNumQuestLogEntries then
    count = C_QuestLog.GetNumQuestLogEntries()
  end
  if type(count) ~= "number" then return end

  local quests, header = {}, nil
  for i = 1, count do
    local title, level, isHeader, complete, failed, questID = questLogRow(i)
    title = plain(title)
    if title then
      if isHeader then
        header = title
      else
        quests[#quests + 1] = {
          id = plain(questID),
          title = title,
          level = plain(level),
          header = header,
          complete = plain(complete) and true or false,
          failed = plain(failed) and true or false,
          objectives = readObjectives(plain(questID), i),
        }
      end
    end
  end
  char.quests = quests
  session.questsDirty = false
  session.questsReadAt = GetTime and GetTime() or now()
end

local function questTitle(questID)
  for _, q in ipairs(char.quests or {}) do
    if q.id == questID then return q.title end
  end
  if C_QuestLog and C_QuestLog.GetTitleForQuestID then
    local t = plain(C_QuestLog.GetTitleForQuestID(questID))
    if t then return t end
  end
  if C_QuestLog and C_QuestLog.GetQuestInfo then
    return plain(C_QuestLog.GetQuestInfo(questID))
  end
end

local function updatePosition()
  if not char then return end
  char.zone = plain(GetRealZoneText())
  char.subzone = plain(GetSubZoneText())
  local inInstance, instanceType = IsInInstance()
  char.instance = (inInstance and instanceType) or nil

  local mapID = plain(C_Map.GetBestMapForUnit("player"))
  local x, y
  if mapID then
    local pos = C_Map.GetPlayerMapPosition(mapID, "player") -- nil inside instances
    if pos then x, y = pos:GetXY() end
    x, y = plain(x), plain(y)
  end

  if mapID and x and y and (x ~= 0 or y ~= 0) then
    x, y = round(x, 4), round(y, 4)
    char.mapID, char.x, char.y = mapID, x, y
    char.hasPosition = true
    char.positionAt = now()
    addTrailPoint(mapID, x, y)
  else
    -- Keep the last known position; just flag that it is not current.
    char.hasPosition = false
  end
end

local function updateXP()
  if not char then return end
  local level = UnitLevel("player")
  local xp, xpMax = UnitXP("player"), UnitXPMax("player")
  if session.level then
    local gained = 0
    if level == session.level and xp > session.xp then
      gained = xp - session.xp
    elseif level > session.level then
      gained = (session.xpMax - session.xp) + xp
    end
    char.xpGainedSession = (char.xpGainedSession or 0) + gained
  end
  session.level, session.xp, session.xpMax = level, xp, xpMax

  char.level, char.xp, char.xpMax = level, xp, xpMax
  char.rested = GetXPExhaustion() or 0
  char.resting = IsResting() and true or false
  sampleXP()
end

-- Snapshot of a unit. In combat, Forever hides NPC name, type and health, so
-- those fields can be missing.
local function readUnit(unit)
  if not UnitExists(unit) then return nil end
  local t = {
    name = plain((UnitName(unit))),
    level = plain(UnitLevel(unit)),
    classification = plain(UnitClassification(unit)),
    creatureType = plain(UnitCreatureType(unit)),
    reaction = plain(UnitReaction(unit, "player")), -- 1-3 hostile, 4 neutral, 5-8 friendly
    isPlayer = plain(UnitIsPlayer(unit)) and true or false,
    dead = plain(UnitIsDead(unit)) and true or false,
  }
  local hp, hpMax = plain(UnitHealth(unit)), plain(UnitHealthMax(unit))
  if hp and hpMax and hpMax > 0 then
    t.hpPct = math.floor(hp / hpMax * 100 + 0.5)
  end
  return t
end

-- Fill hidden fields from an earlier snapshot that looks like the same unit.
local function fillFrom(fresh, prev)
  if not (fresh and prev) then return fresh end
  if fresh.level == prev.level and fresh.classification == prev.classification then
    fresh.name = fresh.name or prev.name
    fresh.creatureType = fresh.creatureType or prev.creatureType
  end
  return fresh
end

local function updateTarget()
  if not char then return end
  char.target = fillFrom(readUnit("target"), char.target)
end

local function onCombatStart()
  char.inCombat = true
  updateTarget()
  char.lastFight = { target = char.target, startedAt = now() }
end

local function onCombatEnd()
  char.inCombat = false
  local fight = char.lastFight
  if fight then
    fight.endedAt = now()
    -- Out of combat the target's name and health are readable again.
    local t = readUnit("target")
    local ft = fight.target
    if t then
      if not ft or ft.level == nil then
        fight.target = t
      elseif ft.level == t.level and ft.classification == t.classification
          and (ft.name == nil or ft.name == t.name) then
        ft.name = t.name or ft.name
        ft.creatureType = t.creatureType or ft.creatureType
        ft.dead, ft.hpPct = t.dead, t.hpPct
      end
    end
  end
  updateTarget()
end

local function initCharacter()
  local name, realm = UnitName("player"), GetRealmName()
  local key = name .. "-" .. realm
  db.characters[key] = db.characters[key] or {}
  char = db.characters[key]

  local className, classFile = UnitClass("player")
  char.key, char.name, char.realm = key, name, realm
  char.class, char.classFile = className, classFile
  -- Lets the companion match combat log lines to this character.
  char.guid = plain(UnitGUID("player"))
  char.race = UnitRace("player")
  char.faction = UnitFactionGroup("player")
  char.status = "online"
  char.loginAt = now()
  char.xpGainedSession = 0
  char.inCombat = UnitAffectingCombat("player") and true or false
  char.trail = char.trail or {}
  char.journal = char.journal or {}
  char.xpSamples = char.xpSamples or {}
  char.quests = char.quests or {}
  char.deaths = char.deaths or 0
  char.played = char.played or 0

  session = {}
  updateXP()
  updatePosition()
  updateTarget()
  updateVitals()
  updateMoney()
  readQuestLog()
  -- Learn the real played time once; afterwards it is counted here. (This
  -- prints the usual "Total time played" lines in chat that one time.)
  if not char.playedSynced and RequestTimePlayed then RequestTimePlayed() end
end

---------------------------------------------------------------------------
-- Saving
---------------------------------------------------------------------------

local function markReload() reloading = true end

local function syncNow()
  reloading = true
  if C_UI and C_UI.Reload then C_UI.Reload() else ReloadUI() end
end

local function onLogout()
  if char then
    updatePosition()
    updateXP()
    updateVitals()
    updateMoney()
    if session.questsDirty then readQuestLog() end
    char.played = math.floor(char.played or 0)
    sampleXP(true)
    -- PLAYER_LOGOUT also fires on /reload; only a real logout means offline.
    char.status = reloading and "online" or "offline"
    char.savedAt = now()
  end
  db.schema = SCHEMA
  db.addonVersion = VERSION
  db.savedAt = now()
end

---------------------------------------------------------------------------
-- Settings
---------------------------------------------------------------------------

local function describeSettings()
  local s = db.settings
  say("endpoint: " .. (s.endpoint ~= "" and s.endpoint or "(not set, the companion's config file is used)"))
  say("token: " .. (s.token ~= "" and ("set (" .. #s.token .. " characters)") or "(not set)"))
  say("trail: " .. (s.trail and "on" or "off"))
end

local function createOptionsPanel()
  if not (Settings and Settings.RegisterCanvasLayoutCategory) then return end

  local panel = CreateFrame("Frame")
  panel.name = "WoW Tracker"

  local title = panel:CreateFontString(nil, "ARTWORK", "GameFontNormalLarge")
  title:SetPoint("TOPLEFT", 16, -16)
  title:SetText("WoW Tracker")

  local help = panel:CreateFontString(nil, "ARTWORK", "GameFontHighlightSmall")
  help:SetPoint("TOPLEFT", title, "BOTTOMLEFT", 0, -8)
  help:SetWidth(560)
  help:SetJustifyH("LEFT")
  help:SetText("The companion app reads these settings from your saved data and sends your "
    .. "character's state to this address. Changes reach the companion on the next /reload or logout.")

  local function addInput(label, key, y, example)
    local fs = panel:CreateFontString(nil, "ARTWORK", "GameFontNormal")
    fs:SetPoint("TOPLEFT", 16, y)
    fs:SetText(label)

    local box = CreateFrame("EditBox", nil, panel, "InputBoxTemplate")
    box:SetSize(320, 22)
    box:SetPoint("TOPLEFT", 22, y - 18)
    box:SetAutoFocus(false)
    local function save(self)
      db.settings[key] = trim(self:GetText())
      self:ClearFocus()
    end
    box:SetScript("OnShow", function(self) self:SetText(db.settings[key] or "") end)
    box:SetScript("OnEnterPressed", save)
    box:SetScript("OnEditFocusLost", save)
    box:SetScript("OnEscapePressed", function(self) self:ClearFocus() end)

    local ex = panel:CreateFontString(nil, "ARTWORK", "GameFontDisableSmall")
    ex:SetPoint("LEFT", box, "RIGHT", 10, 0)
    ex:SetText(example)
  end

  addInput("Backend address", "endpoint", -80, "e.g. 192.168.0.117:1337")
  addInput("Token (optional)", "token", -130, "must match the backend's ingest_token")

  local trail = CreateFrame("CheckButton", nil, panel, "UICheckButtonTemplate")
  trail:SetPoint("TOPLEFT", 16, -180)
  trail:SetScript("OnShow", function(self) self:SetChecked(db.settings.trail) end)
  trail:SetScript("OnClick", function(self) db.settings.trail = self:GetChecked() and true or false end)
  local trailLabel = panel:CreateFontString(nil, "ARTWORK", "GameFontHighlight")
  trailLabel:SetPoint("LEFT", trail, "RIGHT", 4, 0)
  trailLabel:SetText("Record a trail of where I have been")

  local sync = CreateFrame("Button", nil, panel, "UIPanelButtonTemplate")
  sync:SetSize(220, 24)
  sync:SetPoint("TOPLEFT", 20, -220)
  sync:SetText("Save and sync now (reload)")
  sync:SetScript("OnClick", syncNow)

  optionsCategory = Settings.RegisterCanvasLayoutCategory(panel, panel.name)
  Settings.RegisterAddOnCategory(optionsCategory)
end

local function openOptions()
  if optionsCategory and Settings and Settings.OpenToCategory then
    Settings.OpenToCategory(optionsCategory:GetID())
  else
    describeSettings()
  end
end

local function printHelp()
  say("commands:")
  say("  /wt endpoint <address>  - where the companion sends data, e.g. 192.168.0.117:1337")
  say("  /wt endpoint clear      - use the companion's config file instead")
  say("  /wt token <secret>      - token the backend expects (/wt token clear to remove)")
  say("  /wt trail on|off        - record a breadcrumb trail")
  say("  /wt clear               - delete this character's trail")
  say("  /wt sync                - save now so the website updates (reloads the UI)")
  say("  /wt status              - show what is being tracked")
  say("  /wt config              - open the settings panel")
end

local function printStatus()
  if not char then say("not loaded yet"); return end
  say(string.format("%s, level %s, %s/%s XP", char.key, tostring(char.level), tostring(char.xp), tostring(char.xpMax)))
  if char.hasPosition then
    say(string.format("%s (%s): %.1f, %.1f on map %d", char.zone or "?", char.subzone or "", char.x * 100, char.y * 100, char.mapID))
  else
    say((char.zone or "?") .. ": no map position here (instances hide it)")
  end
  say("trail points: " .. #char.trail .. ", in combat: " .. tostring(char.inCombat))
  say(string.format("quests: %d, journal entries: %d, deaths: %d, played: %d min",
    #char.quests, #char.journal, char.deaths or 0, math.floor((char.played or 0) / 60)))
  describeSettings()
end

SLASH_WOWTRACKER1 = "/wt"
SLASH_WOWTRACKER2 = "/wowtracker"
SlashCmdList.WOWTRACKER = function(msg)
  local cmd, rest = trim(msg):match("^(%S*)%s*(.-)$")
  cmd = (cmd or ""):lower()
  rest = trim(rest)

  if cmd == "endpoint" then
    if rest == "" then
      say("endpoint: " .. (db.settings.endpoint ~= "" and db.settings.endpoint or "(not set)"))
    else
      db.settings.endpoint = (rest:lower() == "clear") and "" or rest
      say("endpoint set to " .. (db.settings.endpoint ~= "" and db.settings.endpoint or "(companion config)")
        .. ". Run /wt sync to save it.")
    end
  elseif cmd == "token" then
    db.settings.token = (rest:lower() == "clear") and "" or rest
    say("token " .. (db.settings.token ~= "" and "set" or "cleared") .. ". Run /wt sync to save it.")
  elseif cmd == "trail" then
    if rest == "on" or rest == "off" then db.settings.trail = (rest == "on") end
    say("trail: " .. (db.settings.trail and "on" or "off"))
  elseif cmd == "clear" then
    if char then char.trail = {} end
    say("trail cleared")
  elseif cmd == "sync" or cmd == "save" then
    syncNow()
  elseif cmd == "status" then
    printStatus()
  elseif cmd == "config" or cmd == "options" then
    openOptions()
  else
    printHelp()
  end
end

---------------------------------------------------------------------------
-- Events
---------------------------------------------------------------------------

local frame = CreateFrame("Frame")

local handlers = {}

function handlers.ADDON_LOADED(name)
  if name ~= ADDON_NAME then return end
  WowTrackerDB = WowTrackerDB or {}
  db = WowTrackerDB
  db.schema = SCHEMA
  db.settings = db.settings or {}
  for k, v in pairs(DEFAULT_SETTINGS) do
    if db.settings[k] == nil then db.settings[k] = v end
  end
  db.characters = db.characters or {}

  if C_UI and type(C_UI.Reload) == "function" then hooksecurefunc(C_UI, "Reload", markReload) end
  if type(ReloadUI) == "function" then hooksecurefunc("ReloadUI", markReload) end
  createOptionsPanel()
  frame:UnregisterEvent("ADDON_LOADED")
end

function handlers.PLAYER_LOGIN()
  initCharacter()
end

function handlers.PLAYER_ENTERING_WORLD(isInitialLogin)
  updatePosition()
  updateXP()
  if isInitialLogin then
    journal("login", { zone = char.zone, level = char.level })
    char.journalZone = char.zone
  end
end

function handlers.ZONE_CHANGED_NEW_AREA()
  updatePosition()
  local zone = char.zone
  if zone and zone ~= "" and zone ~= char.journalZone then
    journal("zone", { zone = zone, level = char.level })
    char.journalZone = zone
  end
end

function handlers.PLAYER_LEVEL_UP(level)
  journal("level", { level = plain(level), zone = char.zone })
  updateXP()
end

function handlers.PLAYER_DEAD()
  char.deaths = (char.deaths or 0) + 1
  journal("death", { level = char.level, zone = char.zone, subzone = char.subzone })
end

-- Classic passes (questLogIndex, questID); newer clients just (questID).
function handlers.QUEST_ACCEPTED(a, b)
  local questID = plain(b or a)
  readQuestLog()
  journal("questAccepted", { questID = questID, title = questTitle(questID) })
end

function handlers.QUEST_TURNED_IN(questID, xp, money)
  questID = plain(questID)
  -- The quest log still lists it until the next refresh.
  journal("questDone", { questID = questID, title = questTitle(questID), xp = plain(xp), money = plain(money) })
  session.questsDirty = true
end

local function questsChanged() session.questsDirty = true end
handlers.QUEST_LOG_UPDATE = questsChanged
handlers.UNIT_QUEST_LOG_CHANGED = questsChanged
handlers.QUEST_REMOVED = questsChanged

handlers.TIME_PLAYED_MSG = onTimePlayed
handlers.PLAYER_MONEY = updateMoney
handlers.UNIT_HEALTH = function(unit)
  if unit == "player" then updateVitals() else updateTarget() end
end
handlers.ZONE_CHANGED = updatePosition
handlers.ZONE_CHANGED_INDOORS = updatePosition
handlers.PLAYER_XP_UPDATE = updateXP
handlers.UPDATE_EXHAUSTION = updateXP
handlers.PLAYER_UPDATE_RESTING = updateXP
handlers.PLAYER_TARGET_CHANGED = updateTarget
handlers.PLAYER_REGEN_DISABLED = onCombatStart
handlers.PLAYER_REGEN_ENABLED = onCombatEnd
handlers.PLAYER_LOGOUT = onLogout

frame:SetScript("OnEvent", function(_, event, ...)
  local handler = handlers[event]
  if handler and (char or event == "ADDON_LOADED" or event == "PLAYER_LOGIN") then
    handler(...)
  end
end)

-- pcall: registering an event this client does not have raises an error.
for event in pairs(handlers) do
  if event == "UNIT_HEALTH" then
    pcall(frame.RegisterUnitEvent, frame, "UNIT_HEALTH", "target", "player")
  elseif event == "UNIT_QUEST_LOG_CHANGED" then
    pcall(frame.RegisterUnitEvent, frame, event, "player")
  else
    pcall(frame.RegisterEvent, frame, event)
  end
end

-- There is no "player moved" event, so poll the position.
local elapsedSince = 0
frame:SetScript("OnUpdate", function(_, elapsed)
  if char then char.played = (char.played or 0) + elapsed end
  elapsedSince = elapsedSince + elapsed
  if elapsedSince < POSITION_INTERVAL then return end
  elapsedSince = 0
  updatePosition()
  if session.questsDirty then
    local t = GetTime and GetTime() or now()
    if t - (session.questsReadAt or 0) >= QUEST_REFRESH then readQuestLog() end
  end
end)
