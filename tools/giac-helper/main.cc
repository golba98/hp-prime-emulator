// SPDX-License-Identifier: GPL-3.0-or-later
// Standalone optional Giac executable. Not linked into the Rust workspace.
#include <giac/config.h>
#include <giac/gen.h>
#include <giac/fraction.h>
#include <giac/usual.h>
#include <cstdint>
#include <iostream>
#include <map>
#include <sstream>
#include <stdexcept>
#include <string>

namespace {
constexpr uint32_t LIMIT = 4 * 1024 * 1024;
size_t consumed = 0;
void read_bytes(char *data, size_t n) {
    consumed += n;
    if (consumed > LIMIT || !std::cin.read(data, n))
        throw std::runtime_error("Truncated or oversized CAS request");
}
uint64_t integer(size_t n) {
    uint64_t value = 0;
    for (size_t i = 0; i < n; ++i) {
        unsigned char c; read_bytes(reinterpret_cast<char *>(&c), 1);
        value |= uint64_t(c) << (8 * i);
    }
    return value;
}
std::string field() {
    uint32_t n = integer(4);
    if (n > LIMIT) throw std::runtime_error("CAS field exceeds 4 MiB");
    std::string out(n, '\0'); read_bytes(out.data(), n); return out;
}
std::string quote(const std::string &s) {
    std::ostringstream out; out << '"';
    const char *hex = "0123456789abcdef";
    for (unsigned char c : s) {
        if (c == '"' || c == '\\') out << '\\' << c;
        else if (c < 32) out << "\\u00" << hex[c >> 4] << hex[c & 15];
        else out << c;
    }
    out << '"'; return out.str();
}
std::string typed(const giac::gen &value, const giac::context *ctx, int depth = 0) {
    if (depth > 64) throw std::runtime_error("CAS result nesting exceeds 64");
    if (giac::is_undef(value) || (value.type == giac::_STRNG && value.subtype == -1))
        throw std::runtime_error(value.print(ctx));
    std::string type, payload;
    switch (value.type) {
    case giac::_INT_:
        // Boolean-subtype integers print true/false; transport their numeric value.
        type = "integer"; payload = quote(std::to_string(value.val)); break;
    case giac::_ZINT:
        type = "integer"; payload = quote(value.print(ctx)); break;
    case giac::_FRAC:
        if (value._FRACptr->num.type > giac::_ZINT || value._FRACptr->den.type > giac::_ZINT) {
            type = "symbolic"; payload = quote(value.print(ctx)); break;
        }
        type = "rational";
        payload = "{\"numerator\":" + quote(value._FRACptr->num.print(ctx)) +
            ",\"denominator\":" + quote(value._FRACptr->den.print(ctx)) + "}";
        break;
    case giac::_DOUBLE_: case giac::_REAL: case giac::_FLOAT_:
        type = "decimal"; payload = quote(value.print(ctx)); break;
    case giac::_CPLX:
        type = "complex";
        payload = "{\"real\":" + typed(value.re(ctx), ctx, depth + 1) +
            ",\"imaginary\":" + typed(value.im(ctx), ctx, depth + 1) + "}";
        break;
    case giac::_STRNG:
        type = "string"; payload = quote(*value._STRNGptr); break;
    case giac::_VECT: {
        const giac::vecteur &values = *value._VECTptr;
        bool matrix = !values.empty() && values[0].type == giac::_VECT;
        size_t cols = matrix ? values[0]._VECTptr->size() : 0;
        for (const auto &row : values)
            matrix = matrix && row.type == giac::_VECT && row._VECTptr->size() == cols;
        payload = "["; bool first = true;
        for (const auto &item : values) {
            if (matrix) {
                for (const auto &cell : *item._VECTptr) {
                    if (!first) payload += ','; first = false;
                    payload += typed(cell, ctx, depth + 1);
                }
            } else {
                if (!first) payload += ','; first = false;
                payload += typed(item, ctx, depth + 1);
            }
        }
        payload += ']';
        if (matrix) {
            type = "matrix";
            payload = "{\"rows\":" + std::to_string(values.size()) +
                ",\"cols\":" + std::to_string(cols) + ",\"values\":" + payload + "}";
        } else type = "list";
        break;
    }
    default:
        type = "symbolic"; payload = quote(value.print(ctx));
    }
    return "{\"type\":" + quote(type) + ",\"value\":" + payload + "}";
}
}
int main() {
    uint64_t request = 0;
    unsigned protocol_version = 1;
    std::ostringstream logs;
    // CAS commands that print must never corrupt the machine protocol.
    auto *protocol = std::cout.rdbuf();
    std::cout.rdbuf(logs.rdbuf());
    try {
        char magic[8]; read_bytes(magic, 8);
        if (std::string(magic, 8) == "PPLCAS02") protocol_version = 2;
        else if (std::string(magic, 8) != "PPLCAS01") throw std::runtime_error("CAS protocol version mismatch");
        request = integer(8);
        auto angle = integer(1), approximate = integer(1), digits = integer(1);
        if (angle > 2 || approximate > 1 || digits == 0 || digits > 100)
            throw std::runtime_error("Invalid CAS settings");
        auto expression = field();
        giac::context ctx;
        giac::secure_run = true;
        giac::angle_mode(int(angle), &ctx);
        giac::decimal_digits(int(digits), &ctx);
        giac::approx_mode(false, &ctx);
        giac::complex_mode(true, &ctx);
        uint64_t count = integer(4);
        if (count > LIMIT / 8) throw std::runtime_error("Too many CAS bindings");
        for (uint64_t i = 0; i < count; ++i) {
            auto name = field(), serialized = field();
            giac::gen parsed(serialized, &ctx);
            if (giac::is_undef(parsed) || (parsed.type == giac::_STRNG && parsed.subtype == -1))
                throw std::runtime_error("Invalid saved CAS binding: " + name);
            // Values are exported Giac expressions. Evaluating them while
            // restoring alphabetically would freeze references to earlier
            // variables and make persistence change algebraic semantics.
            giac::sto(parsed, giac::gen(name, &ctx), &ctx);
        }
        std::map<std::string,std::string> home_before;
        if (protocol_version == 2) {
            uint64_t home_count = integer(4);
            if (home_count > LIMIT / 8) throw std::runtime_error("Too many Home bindings");
            for (uint64_t i = 0; i < home_count; ++i) {
                auto alias = field(), serialized = field();
                if (alias.rfind("pplHomeBridge",0) != 0 || home_before.count(alias) || ctx.tabptr->find(alias.c_str()) != ctx.tabptr->end())
                    throw std::runtime_error("Invalid or duplicate Home binding alias");
                giac::gen parsed(serialized,&ctx);
                if (giac::is_undef(parsed) || (parsed.type == giac::_STRNG && parsed.subtype == -1))
                    throw std::runtime_error("Invalid Home binding expression");
                giac::sto(parsed,giac::gen(alias,&ctx),&ctx);
                auto found=ctx.tabptr->find(alias.c_str());
                if (found==ctx.tabptr->end()) throw std::runtime_error("Home alias was not stored");
                home_before.emplace(alias,found->second.print(&ctx));
            }
        }
        // Read EOF before evaluating; trailing data indicates a bad caller.
        if (std::cin.peek() != std::char_traits<char>::eof())
            throw std::runtime_error("Trailing CAS request bytes");
        giac::approx_mode(approximate != 0, &ctx);
        giac::gen parsed(expression, &ctx);
        giac::gen value = approximate ? giac::evalf(parsed, giac::eval_level(&ctx), &ctx) : giac::eval(parsed, giac::eval_level(&ctx), &ctx);
        auto result = typed(value, &ctx);
        // Capture stored values without evaluating them again: exports must
        // not repeat side effects or change the primary calculation's answer.
        std::string home_updates="{";bool first_home=true;
        for (const auto &home : home_before) {
            auto current=ctx.tabptr->find(home.first.c_str());
            if (current!=ctx.tabptr->end() && current->second.print(&ctx)==home.second) continue;
            if (!first_home) home_updates+=',';first_home=false;
            home_updates+=quote(home.first)+':'+typed(current==ctx.tabptr->end()?giac::gen(0):current->second,&ctx);
        }
        home_updates+='}';
        std::string bindings = "{"; bool first = true;
        for (const auto &binding : *ctx.tabptr) {
            if (!first) bindings += ','; first = false;
            bindings += quote(binding.first) + ':' + quote(binding.second.print(&ctx));
        }
        bindings += '}';
        std::cout.rdbuf(protocol);
        std::cout << "{\"version\":" << protocol_version << ",\"request\":" << request
                  << ",\"engine\":" << quote("giac-" + giac::version()) << ",\"result\":" << result
                  << ",\"error\":null,\"bindings\":" << bindings;
        if (protocol_version==2) std::cout << ",\"home_updates\":" << home_updates;
        std::cout << "}\n";
        return 0;
    } catch (const std::exception &e) {
        std::cout.rdbuf(protocol);
        std::cout << "{\"version\":" << protocol_version << ",\"request\":" << request
                  << ",\"engine\":" << quote("giac-" + giac::version())
                  << ",\"result\":null,\"error\":" << quote(e.what())
                  << ",\"bindings\":{}}\n";
        return 0;
    }
}
