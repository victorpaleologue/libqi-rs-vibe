// qi-dump-values: serialize a fixed catalogue of values with the reference
// libqi binary codec and print one JSON object per line:
//
//   {"name": ..., "signature": ..., "hex": ..., "qi_json": ...}       (values)
//   {"name": ..., "kind": "message", "id": ..., "type": ..., ..., "hex": ...}
//                                                                    (messages)
//
// "hex" is the lowercase hex of the exact bytes libqi would put on the wire
// (sub-buffers of qi::Buffer values are flattened exactly like
// src/messaging/sock/send.hpp does; message dumps include the 28-byte header).
// "qi_json" is qi::encodeJSON of the value (as a string) when libqi can produce
// it (informative only). The output is deterministic: message ids are set explicitly.

#include <boost/optional.hpp>
#include <cstdint>
#include <cstring>
#include <iostream>
#include <map>
#include <string>
#include <utility>
#include <vector>

#include <qi/anyobject.hpp>
#include <qi/anyvalue.hpp>
#include <qi/application.hpp>
#include <qi/binarycodec.hpp>
#include <qi/buffer.hpp>
#include <qi/jsoncodec.hpp>
#include <qi/messaging/serviceinfo.hpp>
#include <qi/objectuid.hpp>
#include <qi/os.hpp>
#include <qi/session.hpp>
#include <qi/signature.hpp>
#include <qi/type/dynamicobject.hpp>
#include <qi/type/dynamicobjectbuilder.hpp>
#include <qi/type/metaobject.hpp>

// Private libqi headers (LIBQI_SOURCE_DIR/src is on the include path).
#include <messaging/message.hpp>
#include <messaging/streamcontext.hpp>

#include "interop_types.hpp"
#include "json.hpp"
#include "stderr_log.hpp"

namespace
{
  /// Flatten a qi::Buffer into the byte sequence the transport sends: the main
  /// buffer with each sub-buffer inlined right after its uint32 size prefix.
  std::vector<std::uint8_t> flatten(const qi::Buffer& buf)
  {
    std::vector<std::uint8_t> out;
    const auto* data = static_cast<const std::uint8_t*>(buf.data());
    size_t begin = 0;
    for (const auto& sub : buf.subBuffers())
    {
      const size_t end = sub.first + sizeof(qi::Buffer::size_type);
      if (data && end > begin)
        out.insert(out.end(), data + begin, data + end);
      begin = end;
      const auto subBytes = flatten(sub.second);
      out.insert(out.end(), subBytes.begin(), subBytes.end());
    }
    if (data && buf.size() > begin)
      out.insert(out.end(), data + begin, data + buf.size());
    return out;
  }

  std::string jsonOf(const qi::AutoAnyReference& ref)
  {
    try
    {
      return qi::encodeJSON(ref);
    }
    catch (const std::exception&)
    {
      return "";
    }
  }

  void emitValue(const std::string& name,
                 const std::string& signature,
                 const qi::Buffer& buf,
                 const std::string& json)
  {
    std::vector<std::pair<std::string, std::string>> fields{
      {"name", ijson::str(name)},
      {"signature", ijson::str(signature)},
      {"hex", ijson::str(ijson::hex(flatten(buf)))},
    };
    // libqi's JSON is informative only and not always strict JSON (e.g. map
    // keys that are not strings), so it is embedded as a string.
    if (!json.empty())
      fields.emplace_back("qi_json", ijson::str(json));
    std::cout << ijson::object(fields) << std::endl;
  }

  void emitError(const std::string& name, const std::string& error)
  {
    std::cout << ijson::object({{"name", ijson::str(name)}, {"error", ijson::str(error)}}) << std::endl;
    std::cerr << "qi-dump-values: " << name << ": " << error << "\n";
  }

  /// Serialize `value` with the type libqi infers for T.
  template <typename T>
  void dump(const std::string& name, const T& value)
  {
    try
    {
      qi::Buffer buf;
      qi::encodeBinary(&buf, value);
      const std::string sig = qi::typeOf<T>()->signature().toString();
      // qi::encodeJSON has no encoder for raw buffers and logs an error.
      emitValue(name, sig, buf, sig.find('r') == std::string::npos ? jsonOf(value) : std::string());
    }
    catch (const std::exception& e)
    {
      emitError(name, e.what());
    }
  }

  /// Serialize the value an AnyReference points to, with the reference's own
  /// type (used for anonymous tuples built at runtime).
  void dumpRef(const std::string& name, const qi::AnyReference& ref)
  {
    try
    {
      qi::Buffer buf;
      qi::encodeBinary(&buf, ref);
      const std::string sig = ref.type()->signature().toString();
      emitValue(name, sig, buf, sig.find('r') == std::string::npos ? jsonOf(ref) : std::string());
    }
    catch (const std::exception& e)
    {
      emitError(name, e.what());
    }
  }

  /// Serialize an AnyValue *as a dynamic* ("m": signature string + value).
  /// Note: passing an AnyValue straight to encodeBinary would serialize its
  /// content instead (AutoAnyReference slices AnyValue to its content).
  void dumpDynamic(const std::string& name, const qi::AnyValue& value)
  {
    dumpRef(name, qi::AnyReference::from(value));
  }

  void dumpMessage(const std::string& name, const qi::Message& msg, const std::string& note = "")
  {
    const qi::Message::Header& h = msg.header();
    std::vector<std::uint8_t> bytes(sizeof(h));
    std::memcpy(bytes.data(), &h, sizeof(h));
    const auto payload = flatten(msg.buffer());
    bytes.insert(bytes.end(), payload.begin(), payload.end());

    std::vector<std::pair<std::string, std::string>> fields{
      {"name", ijson::str(name)},
      {"kind", ijson::str("message")},
      {"id", ijson::num(h.id)},
      {"type", ijson::num(static_cast<unsigned>(h.type))},
      {"type_name", ijson::str(qi::Message::typeToString(msg.type()))},
      {"flags", ijson::num(static_cast<unsigned>(h.flags))},
      {"service", ijson::num(h.service)},
      {"object", ijson::num(h.object)},
      {"action", ijson::num(h.action)},
      {"payload_size", ijson::num(h.size)},
      {"payload_hex", ijson::str(ijson::hex(payload))},
      {"hex", ijson::str(ijson::hex(bytes))},
    };
    if (!note.empty())
      fields.emplace_back("note", ijson::str(note));
    std::cout << ijson::object(fields) << std::endl;
  }

  int addOne(int v) { return v + 1; }
}

int main(int argc, char** argv)
{
  // qi::Application initializes logging, the type system statics etc.
  qi::Application app(argc, argv);
  interop::routeQiLogsToStderr();

  // ---- primitives -------------------------------------------------------
  dump<bool>("bool_true", true);
  dump<bool>("bool_false", false);
  dump<std::int8_t>("int8_neg5", -5);
  dump<std::uint8_t>("uint8_200", 200);
  dump<std::int16_t>("int16_neg1234", -1234);
  dump<std::uint16_t>("uint16_60000", 60000);
  dump<std::int32_t>("int32_neg100000", -100000);
  dump<std::uint32_t>("uint32_3000000000", 3000000000u);
  dump<std::int64_t>("int64_neg5e12", -5000000000000LL);
  dump<std::uint64_t>("uint64_1e19", 10000000000000000000ULL);
  dump<float>("float_1_5", 1.5f);
  dump<double>("double_neg2_25", -2.25);

  // ---- strings ----------------------------------------------------------
  dump<std::string>("string_empty", std::string());
  dump<std::string>("string_abc", std::string("abc"));
  dump<std::string>("string_hello_utf8", std::string("h\xc3\xa9llo")); // "héllo"

  // ---- containers -------------------------------------------------------
  dump("vec_int32_1_2_3", std::vector<std::int32_t>{1, 2, 3});
  dump("vec_string_a_bc", std::vector<std::string>{"a", "bc"});
  dump("vec_int32_empty", std::vector<std::int32_t>{});
  dump("map_string_int32", std::map<std::string, std::int32_t>{{"a", 1}, {"b", 2}});
  dump("map_int32_string", std::map<std::int32_t, std::string>{{1, "a"}});
  {
    // std::tuple is not a libqi type; anonymous tuples are built at runtime.
    const std::int32_t i = 1;
    const std::string s = "a";
    qi::AnyValue tuple = qi::AnyValue::makeTuple({qi::AnyReference::from(i), qi::AnyReference::from(s)});
    dumpRef("tuple_int32_string", tuple.asReference());
  }
  dump("pair_int32_string", std::pair<std::int32_t, std::string>(1, "a"));

  // ---- registered structs -----------------------------------------------
  dump("struct_point2d", Point2D{4, 2});
  dump("struct_nested_timestamped_point2d", TimeStampedPoint2D{Point2D{4, 2}, TimeStamp{3, 1}});

  // ---- dynamics (qi::AnyValue, signature "m") -----------------------------
  dumpDynamic("dynamic_int32_5", qi::AnyValue::from<std::int32_t>(5));
  dumpDynamic("dynamic_string_abc", qi::AnyValue::from<std::string>("abc"));
  dumpDynamic("dynamic_vec_int32_1_2", qi::AnyValue::from(std::vector<std::int32_t>{1, 2}));
  dumpDynamic("dynamic_point2d", qi::AnyValue::from(Point2D{4, 2}));
  dumpDynamic("dynamic_empty", qi::AnyValue());
  dumpDynamic("dynamic_void", qi::AnyValue(qi::typeOf<void>()));
  dumpDynamic("dynamic_bool_true", qi::AnyValue::from<bool>(true));
  dumpDynamic("dynamic_double_1_5", qi::AnyValue::from<double>(1.5));
  dumpDynamic("dynamic_map_string_dynamic",
       qi::AnyValue::from(std::map<std::string, qi::AnyValue>{
         {"i", qi::AnyValue::from<std::int32_t>(1)},
         {"s", qi::AnyValue::from<std::string>("x")}}));
  dump("vec_dynamic_int32_string",
       std::vector<qi::AnyValue>{qi::AnyValue::from<std::int32_t>(1), qi::AnyValue::from<std::string>("a")});

  // ---- optionals ----------------------------------------------------------
  dump("optional_int32_none", boost::optional<std::int32_t>());
  dump("optional_int32_some_7", boost::optional<std::int32_t>(7));
  dump("optional_string_some_abc", boost::optional<std::string>(std::string("abc")));

  // ---- raw buffer -----------------------------------------------------------
  {
    qi::Buffer raw;
    const std::uint8_t bytes[4] = {0x2a, 0, 0, 0};
    raw.write(bytes, sizeof(bytes));
    dump("buffer_4_bytes", raw);
    qi::Buffer empty;
    dump("buffer_empty", empty);
    // A buffer nested inside a tuple: the sub-buffer size prefix sits in the
    // middle of the main buffer.
    const std::int32_t seven = 7;
    const std::string z = "z";
    qi::AnyValue tuple = qi::AnyValue::makeTuple(
      {qi::AnyReference::from(seven), qi::AnyReference::from(raw), qi::AnyReference::from(z)});
    dumpRef("tuple_int32_buffer_string", tuple.asReference());
  }

  // ---- misc registered types ------------------------------------------------
  dump("signature_value_is", qi::Signature("(is)"));
  dump("os_timeval", qi::os::timeval(1700000000, 123456));
  dump("url_tcp_localhost", qi::Url("tcp://127.0.0.1:9559"));

  // ---- MetaObject ---------------------------------------------------------
  {
    // Build with an explicit DynamicObject so the raw (pre-`object()`) meta
    // object can be captured before the Manageable members get merged in.
    auto* dyn = new qi::DynamicObject();
    qi::DynamicObjectBuilder ob(dyn, /*isObjectOwner=*/true);
    ob.advertiseMethod("add", &addOne, std::string("Adds one to its argument"));
    ob.advertiseSignal<int>("fire");
    ob.advertiseProperty<int>("val");
    ob.setDescription("Interop test object");
    const qi::MetaObject rawMo = dyn->metaObject();
    dump("metaobject_builder_raw", rawMo);
    qi::AnyObject obj = ob.object();
    dump("metaobject_full_object", obj.metaObject());
    dump("metaobject_empty", qi::MetaObject());
  }

  // ---- ServiceInfo --------------------------------------------------------
  {
    qi::ServiceInfo si;
    si.setName("Calculator");
    si.setServiceId(2);
    si.setMachineId("9a65b56e-c3d3-4485-8924-661b036202b3");
    si.setProcessId(3420486);
    si.setEndpoints(qi::UrlVector{qi::Url("tcp://127.0.0.1:41681")});
    si.setSessionId("361ecec4-00f7-4c94-a6e2-d91e28c5a06c");
    qi::ObjectUid uid;
    {
      std::uint8_t* p = begin(uid);
      for (std::uint8_t i = 0; i < 20; ++i)
        p[i] = static_cast<std::uint8_t>(i + 1);
    }
    si.setObjectUid(qi::serializeObjectUid<std::string>(uid));
    dump("serviceinfo_calculator", si);

    qi::ServiceInfo sdInfo;
    sdInfo.setName("ServiceDirectory");
    sdInfo.setServiceId(1);
    sdInfo.setMachineId("9a65b56e-c3d3-4485-8924-661b036202b3");
    sdInfo.setProcessId(3420486);
    sdInfo.setEndpoints(qi::UrlVector{qi::Url("tcp://127.0.0.1:9559")});
    sdInfo.setSessionId("0");
    dump("serviceinfo_servicedirectory_no_uid", sdInfo);

    dump("objectuid_raw_20_bytes", uid);
    dump("vec_serviceinfo_one", std::vector<qi::ServiceInfo>{si});
  }

  // ---- CapabilityMap --------------------------------------------------------
  const qi::CapabilityMap defaultCaps = qi::StreamContext::defaultCapabilities();
  dump("capabilitymap_default", defaultCaps);
  {
    qi::CapabilityMap authDone;
    authDone["__qi_auth_state"] = qi::AnyValue::from<unsigned int>(3);
    dump("capabilitymap_auth_state_done", authDone);
  }

  // ---- Messages -------------------------------------------------------------
  {
    qi::Message call;
    call.setType(qi::Message::Type_Call);
    call.setId(1);
    call.setService(qi::Message::Service_Server);
    call.setObject(qi::Message::GenericObject_None);
    call.setFunction(qi::Message::ServerFunction_Authenticate);
    call.setValue(defaultCaps, qi::typeOf<qi::CapabilityMap>()->signature());
    dumpMessage("msg_call_authenticate", call,
                "client -> server first message; payload {sm} = default StreamContext capabilities");
  }
  {
    qi::CapabilityMap authDone;
    authDone["__qi_auth_state"] = qi::AnyValue::from<unsigned int>(3);
    qi::Message reply;
    reply.setType(qi::Message::Type_Reply);
    reply.setId(1);
    reply.setService(qi::Message::Service_Server);
    reply.setObject(qi::Message::GenericObject_None);
    reply.setFunction(qi::Message::ServerFunction_Authenticate);
    reply.setValue(authDone, qi::typeOf<qi::CapabilityMap>()->signature());
    dumpMessage("msg_reply_authenticate_state_only", reply, "Server::sendSuccessfulAuthReply shape");

    qi::CapabilityMap full = defaultCaps;
    full["__qi_auth_state"] = qi::AnyValue::from<unsigned int>(3);
    qi::Message replyFull;
    replyFull.setType(qi::Message::Type_Reply);
    replyFull.setId(1);
    replyFull.setService(qi::Message::Service_Server);
    replyFull.setObject(qi::Message::GenericObject_None);
    replyFull.setFunction(qi::Message::ServerFunction_Authenticate);
    replyFull.setValue(full, qi::typeOf<qi::CapabilityMap>()->signature());
    dumpMessage("msg_reply_authenticate_with_caps", replyFull,
                "Server::sendAuthReply shape: __qi_auth_state=3 merged with the server capabilities");
  }
  {
    qi::Message err;
    err.setType(qi::Message::Type_Error);
    err.setId(5);
    err.setService(2);
    err.setObject(1);
    err.setFunction(100);
    err.setError("boom");
    dumpMessage("msg_error_boom", err, "error payload is a dynamic (m) holding the string");
  }
  {
    qi::Message cancel;
    cancel.setType(qi::Message::Type_Cancel);
    cancel.setId(9);
    cancel.setService(2);
    cancel.setObject(1);
    const unsigned int originalId = 7;
    cancel.setValue(originalId, "I");
    dumpMessage("msg_cancel_call_7", cancel,
                "RemoteObject::onFutureCancelled: new id, action stays 0, payload = uint32 id of the call");
  }
  {
    qi::Message canceled;
    canceled.setType(qi::Message::Type_Canceled);
    canceled.setId(7);
    canceled.setService(2);
    canceled.setObject(1);
    canceled.setFunction(100);
    dumpMessage("msg_canceled_call_7", canceled, "answer to the canceled call: same id/address, empty payload");
  }
  {
    qi::Message event;
    event.setType(qi::Message::Type_Event);
    event.setId(11);
    event.setService(2);
    event.setObject(1);
    event.setEvent(100);
    const std::int32_t v = 42;
    event.setValues({qi::AnyReference::from(v)}, qi::Signature("(i)"));
    dumpMessage("msg_event_int32_42", event, "payload = signal parameter tuple (i)");
  }
  {
    qi::Message call;
    call.setType(qi::Message::Type_Call);
    call.setId(3);
    call.setService(2);
    call.setObject(1);
    call.setFunction(100);
    const std::int32_t a = 1, b = 2;
    call.setValues({qi::AnyReference::from(a), qi::AnyReference::from(b)}, qi::Signature("(ii)"));
    dumpMessage("msg_call_add_1_2", call, "method call with argument tuple (ii)");
  }
  {
    qi::Message reply;
    reply.setType(qi::Message::Type_Reply);
    reply.setId(3);
    reply.setService(2);
    reply.setObject(1);
    reply.setFunction(100);
    const std::int32_t r = 3;
    reply.setValue(r, "i");
    dumpMessage("msg_reply_add_3", reply, "reply carries the call id and address");
  }
  {
    qi::Message call;
    call.setType(qi::Message::Type_Call);
    call.setId(2);
    call.setService(2);
    call.setObject(1);
    call.setFunction(qi::Message::BoundObjectFunction_MetaObject);
    const unsigned int zero = 0;
    call.setValues({qi::AnyReference::from(zero)}, qi::Signature("(I)"));
    dumpMessage("msg_call_metaobject", call, "metaObject::(I) as sent by RemoteObject::fetchMetaObject (argument 0)");
  }
  {
    qi::Message call;
    call.setType(qi::Message::Type_Call);
    call.setId(4);
    call.setService(2);
    call.setObject(1);
    call.setFunction(qi::Message::BoundObjectFunction_RegisterEvent);
    const unsigned int service = 2, event = 100;
    const qi::uint64_t link = (static_cast<qi::uint64_t>(event) << 32) | 2u;
    call.setValues({qi::AnyReference::from(service), qi::AnyReference::from(event), qi::AnyReference::from(link)},
                   qi::Signature("(IIL)"));
    dumpMessage("msg_call_registerevent", call, "registerEvent::(IIL) -> L; link = (eventId << 32) | counter");
  }
  {
    qi::Message post;
    post.setType(qi::Message::Type_Post);
    post.setId(6);
    post.setService(2);
    post.setObject(1);
    post.setFunction(101);
    const std::string s = "abc";
    post.setValues({qi::AnyReference::from(s)}, qi::Signature("(s)"));
    dumpMessage("msg_post_string_abc", post, "post (no reply expected) with argument tuple (s)");
  }
  {
    qi::Message dyn;
    dyn.setType(qi::Message::Type_Call);
    dyn.setId(8);
    dyn.setService(2);
    dyn.setObject(1);
    dyn.setFunction(100);
    dyn.addFlags(qi::Message::TypeFlag_DynamicPayload);
    const std::int32_t a = 1;
    const std::string b = "a";
    dyn.setValues({qi::AnyReference::from(a), qi::AnyReference::from(b)}, qi::Signature("m"));
    dumpMessage("msg_call_dynamic_payload", dyn,
                "TypeFlag_DynamicPayload: payload is a dynamic wrapping the argument tuple (is)");
  }
  {
    qi::Message caps;
    caps.setType(qi::Message::Type_Capability);
    caps.setId(10);
    caps.setService(qi::Message::Service_Server);
    caps.setObject(qi::Message::GenericObject_None);
    caps.setFunction(0);
    caps.setValue(defaultCaps, qi::typeOf<qi::CapabilityMap>()->signature());
    dumpMessage("msg_capability_default", caps, "Type_Capability message (service 0, object 0, action 0)");
  }

  std::cout.flush();
  return 0;
}
