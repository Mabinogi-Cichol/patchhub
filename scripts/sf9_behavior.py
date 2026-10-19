"""Static lowering of bounded Lua AST into read-only installation predicates.

No source is executed. Values are literals, UI-only records, function syntax or
symbolic file predicates. Unknown effects/calls/loops cannot grant permission.
"""
import re
from sf95_audit import require, safe_target
from sf9_lua import parse

VERSION = "sf9-behavior-v3"
ROOT = "X:\\__scanned_game__"


def scope_copy(env):
    # Branch-local UI records and tracking sets must not mutate sibling paths.
    return {key: value.copy() if isinstance(value, (dict, set)) else value for key, value in env.items()}


def symbol(kind, *values): return ("symbol", kind, *values)
def symbolic(value): return isinstance(value, tuple) and value and value[0] == "symbol"
def truth(value):
    if symbolic(value): return value
    return value is not False and value is not None

def boolean(op, left, right=None):
    if op == "not":
        if type(left) is bool: return not left
        return symbol("not", left)
    if op == "and":
        if left is False or right is False: return False
        if left is True: return right
        if right is True: return left
    if op == "or":
        if left is True or right is True: return True
        if left is False: return right
        if right is False: return left
    if left == right: return left
    return symbol(op, left, right)


def file_policy(files, version):
    """Validate fields used by member selection; preferences are data."""
    for row in files:
        require(isinstance(row["destination"], str) and isinstance(row["condition"], str)
                and isinstance(row["packages"], list), "exe_file_schema_unsupported")


class Compiler:
    def __init__(self, total):
        self.steps = 0
        self.depth = 0
        self.next_paths = []
        self.mode = "project"
        self.event_name = "Global Functions"
        self.protected = set()
        self.packages = {}
        self.lifecycle = "global"
        self.env = {"_NeedsReboot": False, "_SilentInstall": False, "_DoingUninstall": False,
                    "_SpaceRequired": total, "_SpaceAvailable": 2**63,
                    "strInstallFolderPath": ROOT, "e_Details": {"Text": ROOT},
                    "e_CtrlID": symbol("ui", "control"), "e_MsgID": symbol("ui", "message"),
                    "e_Stage": 2, "e_StagePct": symbol("ui", "stage-percent"),
                    "e_CurrentItemText": symbol("ui", "item-text"), "e_TotalPct": symbol("ui", "total-percent")}

    def tick(self):
        self.steps += 1
        require(self.steps <= 200000, "exe_lua_analysis_limit")

    def value(self, node, env, path):
        self.tick()
        kind = node[0]
        if kind == "literal": return node[1]
        if kind == "name":
            if node[1] in env: return env[node[1]]
            if node[1] in {"File", "Registry", "SessionVar", "Screen", "SetupData", "Dialog", "String", "string", "Drive", "Application", "System"} or node[1].startswith("Dlg"):
                return ("namespace", node[1])
            raise ValueError("exe_lua_unknown_value:" + node[1])
        if kind == "function": return node
        if kind == "index":
            parent, key = self.value(node[1], env, path), self.value(node[2], env, path)
            if isinstance(parent, tuple) and parent[0] == "namespace" and isinstance(key, str):
                return ("api", parent[1] + "." + key)
            require(isinstance(parent, dict) and key in parent, "exe_lua_index_unsupported")
            return parent[key]
        if kind == "table": return {self.value(k, env, path): self.value(v, env, path) for k, v in node[1]}
        if kind == "unary":
            value = self.value(node[2], env, path)
            if node[1] == "not": return boolean("not", truth(value))
            require(not symbolic(value), "exe_lua_arithmetic_unsupported")
            if node[1] == "-": return -value
            if node[1] == "#": return len(value)
        if kind == "binary":
            op = node[1]
            left = self.value(node[2], env, path)
            if op == "and" and truth(left) is False: return left
            if op == "or" and truth(left) is True: return left
            right = self.value(node[3], env, path)
            if op in ("and", "or"):
                # Lua returns an operand, not its boolean conversion.
                if not symbolic(left): return right
                require(type(right) is bool or symbolic(right), "exe_lua_operand_result_unsupported")
                return boolean(op, truth(left), truth(right))
            if symbolic(left) or symbolic(right):
                if op in ("==", "~="):
                    for candidate, literal in ((left, right), (right, left)):
                        if symbolic(candidate) and candidate[1] != "crc32" and type(literal) is bool:
                            return candidate if (literal == (op == "==")) else boolean("not", candidate)
                require(op in ("==", "~=", "<", ">", "<=", ">="), "exe_lua_arithmetic_unsupported")
                return symbol(op, left, right)
            if op == "..":
                require(isinstance(left, (str, int)) and isinstance(right, (str, int)), "exe_lua_concat_unsupported")
                return str(left) + str(right)
            if op == "==": return type(left) == type(right) and left == right
            if op == "~=": return type(left) != type(right) or left != right
            if op == "<": return left < right
            if op == ">": return left > right
            if op == "<=": return left <= right
            if op == ">=": return left >= right
            require(type(left) in (int, float) and type(right) in (int, float), "exe_lua_arithmetic_unsupported")
            if op == "+": return left + right
            if op == "-": return left - right
            if op == "*": return left * right
            if op == "/": return left / right
            if op == "%": return left % right
            raise ValueError("exe_lua_arithmetic_unsupported")
        if kind == "call":
            target = self.value(node[1], env, path)
            args = [self.value(a, env, path) for a in node[2]]
            if isinstance(target, tuple) and target[0] == "api": return self.api(target[1], args, env, path)
            require(isinstance(target, tuple) and target[0] == "function", "exe_lua_dynamic_call_unsupported")
            require(len(args) == len(target[1]), "exe_lua_argument_unsupported")
            self.depth += 1
            require(self.depth <= 32, "exe_lua_call_depth_limit")
            child = scope_copy(env)
            child.update(zip(target[1], args))
            child["_locals"] = set(target[1])
            child["_writes"] = set()
            states = self.block(target[2], [(child, path, False, None)])
            self.depth -= 1
            writes = set().union(*(s[0].get("_writes", set()) for s in states))
            for key in writes:
                observed = [s[0].get(key) for s in states]
                require(all(value == observed[0] for value in observed), "exe_lua_global_mutation_unsupported")
                env[key] = observed[0]
                if self.depth and key not in env.get("_locals", set()): env.setdefault("_writes", set()).add(key)
            values = [s[3] for s in states]
            require(all(v == values[0] for v in values), "exe_lua_function_result_unsupported")
            return values[0]
        raise ValueError("exe_lua_expression_unsupported")

    def api(self, name, args, env, path):
        if name in ("File.DoesExist", "File.GetCRC"):
            require(len(args) == 1 and isinstance(args[0], str), "exe_file_condition_unsupported")
            value = args[0]
            if value.startswith("%AppFolder%\\"):
                relative = value[len("%AppFolder%\\"):]
            elif value.startswith(ROOT + "\\"):
                relative = value[len(ROOT) + 1:]
            elif self.mode == "locator" and name == "File.DoesExist":
                # A checked directory-probe argument is replaced by the scanned
                # instance; only its literal filename can become a condition.
                relative = value.rsplit("\\", 1)[-1]
            else: raise ValueError("exe_condition_path_unsupported")
            relative = relative.replace("/", "\\")
            parent, _, leaf = relative.rpartition("\\")
            relative = safe_target("%AppFolder%" + ("\\" + parent if parent else ""), leaf)
            return symbol("exists" if name.endswith("DoesExist") else "crc32", relative)
        if name == "Registry.GetValue":
            require(self.mode == "locator" and len(args) in (3, 4), "exe_registry_read_unsupported")
            return ROOT
        if name == "SessionVar.Set":
            require(len(args) == 2 and args[0] in ("%AppFolder%", "%SpaceRequired%", "%SpaceAvailable%"), "exe_variable_write_unsupported")
            if args[0] == "%AppFolder%":
                require(self.mode == "locator" and isinstance(args[1], str), "exe_target_change_unsupported")
            return None
        if name == "SessionVar.Expand": return args[0].replace("%AppFolder%", ROOT) if isinstance(args[0], str) else args[0]
        if name == "Screen.Next":
            require(self.event_name == "On Next", "exe_screen_transition_unsupported")
            self.next_paths.append(path)
            return None
        if name == "SetupData.GetPackageProperties":
            require(args and isinstance(args[0], str), "exe_package_name_missing")
            require(args[0] in self.packages, "exe_package_not_found")
            return {"Install": self.packages[args[0]]}
        if name == "SetupData.SetPackageProperties":
            choosing = (self.lifecycle == "global"
                or self.lifecycle == "before" and self.event_name in ("On Preload", "On Next")
                or self.lifecycle == "project" and self.event_name in ("On Startup", "On Pre Install")
                or self.lifecycle == "while" and self.event_name == "On Preload")
            if not choosing: return None
            require(args and isinstance(args[0], str), "exe_package_name_missing")
            require(args[0] in self.packages, "exe_package_not_found")
            require(len(args) > 1 and isinstance(args[1], dict), "exe_package_properties_invalid")
            if "Install" in args[1]:
                old = self.packages[args[0]]
                self.packages[args[0]] = boolean("or", boolean("and", path, truth(args[1]["Install"])), boolean("and", boolean("not", path), old))
            return None
        if name == "Screen.Back": return None
        if name == "Application.Exit":
            require(self.mode == "cancel" and args == [env.get("EXIT_REASON_USER_ABORTED")], "exe_application_exit_unsupported")
            return None
        if name in ("Screen.GetLocalizedString", "SetupData.GetLocalizedString"): return ""
        if name == "SetupData.CalculateRequiredSpace": return env["_SpaceRequired"]
        if name == "Drive.GetFreeSpace": return 2**43
        if name == "Dialog.Message": return env.get("IDNO", 7)
        if name == "Dialog.FolderBrowse": return ROOT
        if name == "String.GetFormattedSize": return ""
        if name == "String.AbbreviateFilePath": return args[0]
        if name == "String.Length":
            require(len(args) == 1 and isinstance(args[0], str), "exe_lua_dynamic_string_unsupported")
            return len(args[0])
        if name == "string.find":
            require(len(args) in (2, 3) and args[0] == ROOT and isinstance(args[1], str), "exe_string_pattern_unsupported")
            pattern = args[1]
            if pattern == r'^\\[^/%*%?"<>|:\][^/%*%?"<>|:]+$': return None
            if pattern == r'^%a:\[^/%*%?"<>|:]*$': return 1
            if pattern in ("\\\\", '[/%*%?"<>|]', '[/%*%?"<>|:]'): return None
            raise ValueError("exe_string_pattern_unsupported")
        if name in ("DlgStaticText.GetProperties", "DlgEditField.GetProperties", "DlgRadioButton.GetProperties"):
            return {"Text": ROOT if "EditField" in name else "", "Checked": True}
        if name in ("DlgStaticText.SetProperties", "DlgButton.SetProperties", "DlgEditField.SetProperties", "DlgProgressBar.SetPos"):
            return None
        raise ValueError("exe_lua_call_unsupported:" + name)

    def block(self, statements, states):
        for statement in statements:
            output = []
            for env, path, ended, result in states:
                self.tick()
                if ended: output.append((env, path, ended, result)); continue
                kind = statement[0]
                if kind == "define":
                    require(self.depth == 0, "exe_lua_nested_function_unsupported")
                    require(statement[1] not in env, "exe_lua_function_redefinition_unsupported")
                    env[statement[1]] = statement[2]
                elif kind == "assign":
                    values = [self.value(n, env, path) for n in statement[2]]
                    for index, target in enumerate(statement[1]):
                        value = values[index] if index < len(values) else None
                        if target[0] == "name":
                            require(target[1] not in ("File", "Registry", "Screen", "SessionVar", "SetupData", "System"), "exe_lua_api_redefinition_unsupported")
                            # Events are analyzed independently. They cannot
                            # silently alter shared constants/runtime guards
                            # used by another event's proof.
                            if target[1] in self.protected:
                                if target[1] == "_SpaceAvailable":
                                    require(type(value) is int and value >= self.env["_SpaceRequired"], "exe_lua_runtime_mutation_unsupported")
                                else:
                                    require(value == self.env[target[1]], "exe_lua_runtime_mutation_unsupported")
                            env[target[1]] = value
                            if self.depth:
                                if statement[3]: env.setdefault("_locals", set()).add(target[1])
                                elif target[1] not in env.get("_locals", set()): env.setdefault("_writes", set()).add(target[1])
                        elif target[0] == "index":
                            parent = self.value(target[1], env, path)
                            key = self.value(target[2], env, path)
                            require(isinstance(parent, dict) and key in ("Text", "Checked", "Enabled", "Install"), "exe_lua_assignment_unsupported")
                            parent[key] = value
                        else: raise ValueError("exe_lua_assignment_unsupported")
                elif kind == "call": self.value(statement[1], env, path)
                elif kind == "return":
                    require(len(statement[1]) <= 1, "exe_lua_multiple_return_unsupported")
                    result = self.value(statement[1][0], env, path) if statement[1] else None
                    ended = True
                elif kind == "if":
                    pending = [(env, path, False, None)]
                    for test, body in statement[1]:
                        remaining = []
                        for scope, condition, _, _ in pending:
                            value = truth(self.value(test, scope, condition))
                            if value is not False:
                                branch = boolean("and", condition, value)
                                output.extend(self.block(body, [(scope_copy(scope), branch, False, None)]))
                            if value is not True:
                                remaining.append((scope_copy(scope), boolean("and", condition, boolean("not", value)), False, None))
                        pending = remaining
                    output.extend(self.block(statement[2], pending))
                    continue
                elif kind == "do":
                    output.extend(self.block(statement[1], [(scope_copy(env), path, False, None)])); continue
                else: raise ValueError("exe_lua_control_flow_unsupported:" + kind)
                output.append((env, path, ended, result))
            states = output
            require(len(states) <= 1024, "exe_lua_branch_limit")
        return states


def lower(graph, files, version):
    require(graph["coverage"]["complete"], "exe_serialized_object_graph_incomplete")
    file_policy(files, version)
    compiler = Compiler(sum(f["unpacked_size"] for f in files))
    compiler.packages = {o["name"]: o["install"] for o in graph.get("objects", []) if o["class"] == "CSUFPackage"}
    global_nodes = []
    for include in graph["includes"]:
        global_nodes.extend(parse(include["script"]))
    for event in graph["events"]:
        if event["name"] == "Global Functions": global_nodes.extend(parse(event["script"]))
    states = compiler.block(global_nodes, [(compiler.env, True, False, None)])
    require(len(states) == 1 and not states[0][2], "exe_lua_global_flow_unsupported")
    compiler.protected = set(compiler.env)
    gates = []
    def lifecycle(event):
        return event.get("lifecycle", "uninstall" if event["context"] == "uninstall" else "project" if event["context"] == "project" else "before")
    def phase(pair):
        i, e = pair; stage, name = lifecycle(e), e["name"]
        order = (0 if stage == "project" and name == "On Startup" else 1 if stage == "before" else
                 2 if stage == "project" and name == "On Pre Install" else 3 if stage == "while" and name == "On Preload" else
                 4 if stage == "while" else 5 if stage == "project" and name == "On Post Install" else 6 if stage == "after" else 7)
        return order, i
    events = [(i,e) for i,e in enumerate(graph["events"]) if lifecycle(e) != "uninstall" and e["name"] != "Global Functions"]
    for _, event in sorted(events, key=phase):
        compiler.lifecycle = lifecycle(event)
        nodes = parse(event["script"])
        compiler.next_paths = []
        compiler.event_name = event["name"]
        context, name = event["context"], event["name"]
        # Locator classification is structural: only this kind of welcome
        # screen can substitute Windows directory probing for a scanned root.
        compiler.mode = "locator" if context.startswith("CSUFScreenStaticText:") and name == "On Next" else "cancel" if name == "On Cancel" else "event"
        # Progress events can be called at every stage; inspecting only the
        # file-copy stage would miss effects in other reachable branches.
        for stage in (1, 2, 3, 4) if name == "On Progress" else (compiler.env["e_Stage"],):
            scope = scope_copy(compiler.env)
            scope["e_Stage"] = stage
            compiler.block(nodes, [(scope, True, False, None)])
        if name == "On Next" and compiler.lifecycle == "before":
            condition = False
            for path in compiler.next_paths: condition = boolean("or", condition, path)
            require(condition is not False, "exe_installation_never_advances")
            gates.append(condition)
    condition = True
    for gate in gates: condition = boolean("and", condition, gate)
    def validate(node):
        if type(node) is bool: return
        require(symbolic(node), "exe_condition_ir_invalid")
        kind = node[1]
        if kind in ("and", "or"):
            validate(node[2]); validate(node[3])
        elif kind == "not": validate(node[2])
        elif kind == "exists": require(isinstance(node[2], str), "exe_condition_ir_invalid")
        elif kind in ("==", "~="):
            operands = node[2:]
            require(any(symbolic(x) and x[1] == "crc32" for x in operands)
                    and any(type(x) is int and 0 <= x <= 0xffffffff for x in operands), "exe_condition_ir_unsupported")
        else: raise ValueError("exe_condition_ir_unsupported")
    validate(condition)
    file_conditions = []
    compiler.mode = "event"
    for row in files:
        expression = row["condition"]
        selected = truth(compiler.value(parse("return (" + expression + ")")[0][1][0], scope_copy(compiler.env), True)) if expression.strip() else True
        if row["packages"]:
            installed = False
            for package in row["packages"]:
                require(package in compiler.packages, "exe_package_not_found")
                installed = boolean("or", installed, compiler.packages[package])
            selected = boolean("and", selected, installed)
        if selected is not True:
            validate(selected)
            file_conditions.append({"path": row["destination"], "condition": selected})
    return {"schema_version": 1, "policy": VERSION, "condition": condition,
            "file_conditions": file_conditions, "copy_files": [f["destination"] for f in files],
            "coverage": {"complete": True, "events": len(graph["events"]), "includes": len(graph["includes"]), "steps": compiler.steps}}
