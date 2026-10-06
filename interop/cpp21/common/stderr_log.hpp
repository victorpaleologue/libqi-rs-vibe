#pragma once
// libqi 2.1's console log handler prints to stdout, which the interop programs
// reserve for machine-readable output: route the logs to stderr instead.

#include <iostream>
#include <string>

#include <qi/log.hpp>
#include <qi/os.hpp>

namespace interop
{
  inline void stderrLogHandler(const qi::LogLevel lv,
                               const qi::os::timeval,
                               const char* category,
                               const char* msg,
                               const char* /*file*/,
                               const char* /*fct*/,
                               int /*line*/)
  {
    std::cerr << "[qi " << qi::log::logLevelToString(lv) << "] " << (category ? category : "") << ": "
              << (msg ? msg : "") << "\n";
  }

  inline void routeQiLogsToStderr(qi::LogLevel level = qi::LogLevel_Warning)
  {
    qi::log::removeLogHandler("consoleloghandler");
    qi::log::addLogHandler("interop-stderr", &stderrLogHandler, level);
  }
}
